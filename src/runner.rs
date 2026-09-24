//! Pipeline runner: drain each provider's seed chains to exhaustion,
//! dedupe across providers, write atomically.
//!
//! `scrape` is the async glue the (sync, object-safe) Provider trait
//! deliberately omits: per seed, fetch -> parse -> advance until the feed
//! ends, capped loudly by max_requests, truncated loudly by time_budget —
//! a partial harvest must never masquerade as an exhausted one. A shared
//! per-provider seen-set stops duplicate fetches across chains and breaks
//! circular advance() loops. One provider crashing costs at most its own
//! share: each chain runs isolated and parser panics are caught.
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use futures::future::join_all;

use crate::fetch::Fetcher;
use crate::model::{ParseKind, Provider, ProxyRecord, Request, Scheme, ScrapeOutcome};
use crate::parse;

pub fn dispatch(
    parser: ParseKind,
    body: &str,
    default: Option<Scheme>,
    source: &'static str,
) -> Vec<ProxyRecord> {
    match parser {
        ParseKind::Entries => parse::entries(body, default, source),
        ParseKind::Json => parse::json(body, default, source),
        ParseKind::Ndjson => parse::ndjson(body, default, source),
        ParseKind::Auto => parse::auto(body, default, source),
        ParseKind::Custom(f) => f(body, default),
    }
}

fn page_cap_hit(record_count: usize, cap: usize) -> bool {
    record_count >= cap
}

fn advance_result(
    result: Result<Option<Request>, String>,
    label: &str,
) -> (Option<Request>, bool, Option<String>) {
    match result {
        Ok(next) => (next, false, None),
        Err(error) => (
            None,
            true,
            Some(format!("{label}: {error} — truncated, not exhausted")),
        ),
    }
}

fn advance_panic_result(label: &str) -> (Option<Request>, bool, Option<String>) {
    // A panic stops before exhaustion just like malformed pagination metadata.
    (None, true, Some(format!("{label}: advance panicked")))
}

async fn drain(
    provider: &dyn Provider,
    fetcher: &Fetcher,
    seed: Request,
    seen: Arc<Mutex<HashSet<String>>>,
    cancelled: &AtomicBool,
) -> (Vec<ProxyRecord>, Vec<String>, usize, usize, bool) {
    let id = provider.id();
    let mut proxies = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut truncated = false;
    let mut req = Some(seed);
    let (mut made, mut ok) = (0usize, 0usize);
    let deadline = Instant::now() + provider.time_budget();

    while let Some(r) = req {
        if should_stop(cancelled.load(Ordering::Relaxed)) {
            truncated = true;
            errors.push(format!(
                "{}: harvest interrupted — chain stopped before starting another request",
                r.label
            ));
            break;
        }
        if made >= provider.max_requests() {
            truncated = true;
            errors.push(format!(
                "{}: stopped at max_requests={} — feed may be larger (truncated, not exhausted)",
                r.label,
                provider.max_requests()
            ));
            break;
        }
        if Instant::now() >= deadline {
            truncated = true;
            errors.push(format!(
                "{}: provider time budget ({}s) reached — partial harvest kept; deep feeds need --proxy-url",
                r.label,
                provider.time_budget().as_secs()
            ));
            break;
        }
        made += 1;
        let label = if r.label.is_empty() {
            r.url.clone()
        } else {
            r.label.clone()
        };
        let Some(body) = fetcher
            .get(&r.url, &r.headers, Some(deadline), cancelled)
            .await
        else {
            match fetch_stop(
                cancelled.load(Ordering::Relaxed),
                Instant::now() >= deadline,
            ) {
                Some(FetchStop::Cancelled) => {
                    truncated = true;
                    errors.push(format!(
                        "{}: harvest interrupted — chain stopped before starting another request",
                        label
                    ));
                }
                Some(FetchStop::Deadline) => {
                    truncated = true;
                    errors.push(format!(
                        "{}: provider time budget ({}s) reached — partial harvest kept; deep feeds need --proxy-url",
                        label,
                        provider.time_budget().as_secs()
                    ));
                }
                None => errors.push(format!("{label}: fetch failed — chain stopped")),
            }
            break;
        };
        // a malformed page (or a provider parser panic) must not sink the run
        let found = catch_unwind(AssertUnwindSafe(|| dispatch(r.parser, &body, r.scheme, id)));
        let mut found = match found {
            Ok(v) => v,
            Err(_) => {
                errors.push(format!("{label}: parser panicked"));
                break;
            }
        };
        let page_record_count = found.len();
        for rec in &mut found {
            if rec.source.is_empty() {
                rec.source = id; // shared parsers don't know the provider
            }
        }
        proxies.extend(found);
        ok += 1;
        if page_cap_hit(page_record_count, parse::MAX_PAGE_RECORDS) {
            truncated = true;
            errors.push(format!(
                "{label}: page record cap {} hit — truncated, not exhaustive (possible amplification)",
                parse::MAX_PAGE_RECORDS
            ));
            break;
        }
        let next = match catch_unwind(AssertUnwindSafe(|| provider.advance(&r, &body))) {
            Ok(result) => advance_result(result, &label),
            Err(_) => advance_panic_result(&label),
        };
        if next.1 {
            truncated = true;
        }
        if let Some(error) = next.2 {
            errors.push(error);
        }
        req = next.0;
        if let Some(next) = &req {
            let mut set = seen.lock().expect("seen mutex poisoned");
            if !set.insert(next.url.clone()) {
                req = None; // covered by a sibling chain (or circular advance)
            }
        }
    }
    (proxies, errors, ok, made, truncated)
}

fn should_stop(cancelled: bool) -> bool {
    cancelled
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetchStop {
    Cancelled,
    Deadline,
}

fn fetch_stop(cancelled: bool, deadline_elapsed: bool) -> Option<FetchStop> {
    if cancelled {
        Some(FetchStop::Cancelled)
    } else if deadline_elapsed {
        Some(FetchStop::Deadline)
    } else {
        None
    }
}

pub async fn scrape(
    provider: Arc<dyn Provider>,
    fetcher: Arc<Fetcher>,
    cancelled: Arc<AtomicBool>,
) -> (ScrapeOutcome, bool) {
    let id = provider.id();
    let mut outcome = ScrapeOutcome {
        provider_id: id,
        proxies: Vec::new(),
        requests_ok: 0,
        requests_total: 0,
        errors: Vec::new(),
    };
    // requests() itself may panic — isolate it
    let seeds = match catch_unwind(AssertUnwindSafe(|| provider.requests())) {
        Ok(s) => s,
        Err(_) => {
            outcome.errors.push("provider crashed in requests()".into());
            return (outcome, false);
        }
    };
    let seen = Arc::new(Mutex::new(
        seeds
            .iter()
            .map(|s| s.url.clone())
            .collect::<HashSet<String>>(),
    ));
    let chains = join_all(seeds.into_iter().map(|seed| {
        let (provider, fetcher, seen, cancelled) = (
            provider.clone(),
            fetcher.clone(),
            seen.clone(),
            cancelled.clone(),
        );
        async move { drain(provider.as_ref(), &fetcher, seed, seen, cancelled.as_ref()).await }
    }))
    .await;
    let mut truncated = false;
    for (proxies, errors, ok, made, chain_truncated) in chains {
        outcome.proxies.extend(proxies);
        outcome.errors.extend(errors);
        outcome.requests_ok += ok;
        outcome.requests_total += made;
        truncated |= chain_truncated;
    }
    (outcome, truncated)
}

/// A signal is not itself incomplete data: provider state decides whether
/// work was actually cut short, while the existing classifier preserves
/// failure and completeness semantics.
pub fn compute_interrupted_outcome(
    providers: &[ProviderRun],
    records_total: usize,
    records_unique: usize,
    task_failures: usize,
) -> RunOutcome {
    compute_run_outcome(providers, records_total, records_unique, task_failures)
}

/// First occurrence wins; sorted by scheme, then host, port, credentials.
pub fn dedupe(all: impl IntoIterator<Item = ProxyRecord>) -> Vec<ProxyRecord> {
    type DedupeKey = (Scheme, String, u16, Option<String>, Option<String>);
    let mut seen: HashSet<DedupeKey> = HashSet::new();
    let mut out: Vec<ProxyRecord> = Vec::new();
    for p in all {
        let key = (
            p.scheme,
            p.host.clone(),
            p.port,
            p.user.clone(),
            p.pass.clone(),
        );
        if seen.insert(key) {
            out.push(p);
        }
    }
    out.sort_by(|a, b| {
        (a.scheme as u8)
            .cmp(&(b.scheme as u8))
            .then_with(|| a.host.cmp(&b.host))
            .then_with(|| a.port.cmp(&b.port))
            .then_with(|| a.user.as_deref().cmp(&b.user.as_deref()))
            .then_with(|| a.pass.as_deref().cmp(&b.pass.as_deref()))
    });
    out
}

/// Provider precedence, not task completion order, decides duplicate source.
pub fn dedupe_provider_batches(mut batches: Vec<(usize, Vec<ProxyRecord>)>) -> Vec<ProxyRecord> {
    batches.sort_by_key(|(order, _)| *order);
    dedupe(batches.into_iter().flat_map(|(_, records)| records))
}

/// Process-level result. Consumers can distinguish a complete harvest from
/// degraded data without scraping logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    Full,
    Partial,
    Failed,
}

impl RunOutcome {
    pub const fn exit_code(self) -> i32 {
        match self {
            Self::Full => 0,
            Self::Partial => 2,
            Self::Failed => 1,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ArtifactPaths {
    pub proxies: PathBuf,
    pub harvest: PathBuf,
    pub summary: PathBuf,
}

impl ArtifactPaths {
    pub fn from_output(path: &str) -> Self {
        let proxies = PathBuf::from(path);
        let candidate = |suffix: &str| match proxies.file_stem().and_then(|stem| stem.to_str()) {
            Some(stem) => match proxies
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                Some(parent) => parent.join(format!("{stem}{suffix}")),
                None => PathBuf::from(format!("{stem}{suffix}")),
            },
            None => PathBuf::from(format!("proxies{suffix}")),
        };
        let disambiguate = |candidate: PathBuf, suffix: &str| {
            if candidate != proxies {
                return candidate;
            }
            match proxies
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                Some(parent) => {
                    let name = proxies.file_name().unwrap_or_default().to_string_lossy();
                    parent.join(format!("{name}{suffix}"))
                }
                None => PathBuf::from(format!("{}{suffix}", proxies.to_string_lossy())),
            }
        };
        Self {
            harvest: disambiguate(candidate(".jsonl"), ".provenance.jsonl"),
            summary: disambiguate(candidate(".summary.json"), ".summary.json"),
            proxies,
        }
    }
}

fn has_contents(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.len() > 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArtifactReport {
    pub proxies_preserved: bool,
    pub harvest_preserved: bool,
}
/// Writes this run's durable artifacts. An empty harvest never replaces a
/// last-good proxy or provenance file. The summary is written last because it
/// is the commit marker; a summary failure after data lands returns an error
/// that makes the caller exit 1, so consumers never trust an incomplete
/// commit. Data artifacts are gated independently, preserving a mismatched
/// non-empty/empty pair rather than deleting either file.
pub fn write_artifacts(
    paths: &ArtifactPaths,
    proxies: &[ProxyRecord],
    providers: &[ProviderRun],
    outcome: RunOutcome,
    started_at: u64,
    duration_secs: f64,
    records_total: usize,
) -> Result<ArtifactReport, std::io::Error> {
    let summary = |outcome: RunOutcome| {
        let provider_values: Vec<_> = providers
            .iter()
            .map(|provider| {
                serde_json::json!({
                    "id": provider.provider_id,
                    "ok_requests": provider.ok_requests,
                    "total_requests": provider.total_requests,
                    "records": provider.records,
                    "duration_secs": provider.duration_secs,
                    "truncated": provider.truncated,
                    "errors": provider.errors,
                })
            })
            .collect();
        serde_json::json!({
            "started_at": started_at,
            "duration_secs": duration_secs,
            "outcome": outcome.label(),
            "exit_code": outcome.exit_code(),
            "records_total": records_total,
            "records_unique": proxies.len(),
            "providers": provider_values,
        })
    };

    let write_data = || -> Result<ArtifactReport, std::io::Error> {
        let failed = outcome == RunOutcome::Failed;
        let proxies_preserved = failed && has_contents(&paths.proxies);
        let harvest_preserved = failed && has_contents(&paths.harvest);
        if !proxies_preserved {
            write_proxies(&paths.proxies, proxies)?;
        }
        if !harvest_preserved {
            let mut harvest = Vec::new();
            for record in proxies {
                let value = serde_json::json!({
                    "proxy": record.url(),
                    "source": record.source,
                    "run_started_at": started_at,
                });
                harvest.extend_from_slice(&value.to_string().into_bytes());
                harvest.push(b'\n');
            }
            write_atomic(&paths.harvest, &harvest)?;
        }
        Ok(ArtifactReport {
            proxies_preserved,
            harvest_preserved,
        })
    };

    match write_data() {
        Ok(report) => {
            // The summary is the commit marker: an exit or abort before this
            // atomic rename leaves the previous run's self-describing summary.
            if let Err(error) =
                write_atomic(&paths.summary, summary(outcome).to_string().as_bytes())
            {
                eprintln!("artifact commit incomplete — treat this run as failed");
                return Err(error);
            }
            Ok(report)
        }
        Err(error) => {
            // Best effort preserves a truthful failure marker; if this write
            // also fails, the prior run's clearly dated summary remains.
            let _ = write_atomic(
                &paths.summary,
                summary(RunOutcome::Failed).to_string().as_bytes(),
            );
            Err(error)
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProviderRun {
    pub provider_id: &'static str,
    pub ok_requests: usize,
    pub total_requests: usize,
    pub records: usize,
    pub duration_secs: f64,
    pub truncated: bool,
    pub errors: Vec<String>,
}

impl ProviderRun {
    pub fn from_outcome(outcome: &ScrapeOutcome, duration_secs: f64, truncated: bool) -> Self {
        Self {
            provider_id: outcome.provider_id,
            ok_requests: outcome.requests_ok,
            total_requests: outcome.requests_total,
            records: outcome.proxies.len(),
            duration_secs,
            truncated,
            errors: outcome.errors.clone(),
        }
    }

    pub fn failed(provider_id: &'static str, duration_secs: f64, error: &str) -> Self {
        Self {
            provider_id,
            ok_requests: 0,
            total_requests: 0,
            records: 0,
            duration_secs,
            truncated: false,
            errors: vec![error.into()],
        }
    }
}

/// Outcome is independent of persistence: callers decide whether an empty
/// result is allowed to replace an existing artifact.
pub fn compute_run_outcome(
    providers: &[ProviderRun],
    records_total: usize,
    records_unique: usize,
    task_failures: usize,
) -> RunOutcome {
    if records_total == 0 || records_unique == 0 {
        return RunOutcome::Failed;
    }
    let complete = task_failures == 0
        && !providers.is_empty()
        && providers.iter().all(|provider| {
            provider.ok_requests > 0 && provider.errors.is_empty() && !provider.truncated
        });
    if complete {
        RunOutcome::Full
    } else {
        RunOutcome::Partial
    }
}

/// Replace a file only after its complete contents are durable. A failed
/// write must never leave a temp file that can be mistaken for an artifact.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), std::io::Error> {
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp_name = std::ffi::OsString::from(".proxplore-");
    temp_name.push(
        path.file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("artifact")),
    );
    temp_name.push(format!("-{}.tmp", std::process::id()));
    let tmp = dir.join(temp_name);

    let result: std::io::Result<()> = (|| {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        Ok(())
    })();

    if let Err(error) = &result {
        let _ = fs::remove_file(&tmp);
        return Err(std::io::Error::new(error.kind(), error.to_string()));
    }
    if let Err(error) = sync_directory(dir) {
        crate::log::debug(
            "runner",
            format_args!("directory sync after rename failed: {error}"),
        );
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(dir: &Path) -> Result<(), std::io::Error> {
    fs::File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_dir: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

/// One URL per line, atomically and durably. Returns line count.
pub fn write_proxies(path: &Path, proxies: &[ProxyRecord]) -> Result<usize, std::io::Error> {
    let mut contents = Vec::new();
    for record in proxies {
        writeln!(contents, "{}", record.url())?;
    }
    write_atomic(path, &contents)?;
    Ok(proxies.len())
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactPaths, FetchStop, ProviderRun, ProxyRecord, RunOutcome, Scheme,
        advance_panic_result, advance_result, compute_interrupted_outcome, compute_run_outcome,
        dedupe, dedupe_provider_batches, fetch_stop, page_cap_hit, should_stop, write_artifacts,
        write_atomic,
    };
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::{fs, path::PathBuf};

    fn provider(id: &'static str, ok: usize, errors: Vec<&str>) -> ProviderRun {
        ProviderRun {
            provider_id: id,
            ok_requests: ok,
            total_requests: ok,
            records: 1,
            duration_secs: 0.1,
            truncated: false,
            errors: errors.into_iter().map(str::to_owned).collect(),
        }
    }

    #[test]
    fn cancellation_decision_follows_only_the_flag() {
        let cancelled = AtomicBool::new(false);
        assert!(!should_stop(cancelled.load(Ordering::Relaxed)));

        cancelled.store(true, Ordering::Relaxed);
        assert!(should_stop(cancelled.load(Ordering::Relaxed)));
    }

    #[test]
    fn fetch_stop_cancellation_takes_precedence_over_deadline() {
        assert_eq!(fetch_stop(true, true), Some(FetchStop::Cancelled));
    }

    #[test]
    fn fetch_stop_deadline_only_is_deadline() {
        assert_eq!(fetch_stop(false, true), Some(FetchStop::Deadline));
    }

    #[test]
    fn fetch_stop_without_stop_condition_is_genuine_fetch_failure() {
        assert_eq!(fetch_stop(false, false), None);
    }

    #[test]
    fn interrupted_cut_short_is_partial_with_exit_two() {
        let mut providers = [provider("alpha", 1, vec![])];
        providers[0].truncated = true;
        let outcome = compute_interrupted_outcome(&providers, 1, 1, 0);

        assert_eq!(outcome, RunOutcome::Partial);
        assert_eq!(outcome.exit_code(), 2);
    }

    #[test]
    fn interrupted_after_complete_chains_is_full_with_exit_zero() {
        let providers = [provider("alpha", 1, vec![])];
        let outcome = compute_interrupted_outcome(&providers, 1, 1, 0);

        assert_eq!(outcome, RunOutcome::Full);
        assert_eq!(outcome.exit_code(), 0);
    }

    #[test]
    fn interrupted_empty_harvest_fails_with_exit_one() {
        let providers = [provider("alpha", 0, vec![])];
        let outcome = compute_interrupted_outcome(&providers, 0, 0, 0);

        assert_eq!(outcome, RunOutcome::Failed);
        assert_eq!(outcome.exit_code(), 1);
    }

    #[test]
    fn outcome_is_full_only_when_every_provider_succeeds() {
        let providers = [provider("alpha", 1, vec![]), provider("beta", 1, vec![])];

        assert_eq!(compute_run_outcome(&providers, 2, 2, 0), RunOutcome::Full);
    }

    #[test]
    fn zero_ok_provider_makes_a_nonempty_run_partial() {
        let providers = [provider("alpha", 1, vec![]), provider("beta", 0, vec![])];

        assert_eq!(
            compute_run_outcome(&providers, 1, 1, 0),
            RunOutcome::Partial
        );
    }

    #[test]
    fn zero_total_records_fails_regardless_of_provider_requests() {
        let providers = [provider("alpha", 1, vec![])];

        assert_eq!(compute_run_outcome(&providers, 0, 0, 0), RunOutcome::Failed);
    }

    #[test]
    fn structured_truncation_without_errors_is_partial() {
        let mut providers = [provider("alpha", 1, vec![])];
        providers[0].truncated = true;

        assert_eq!(
            compute_run_outcome(&providers, 1, 1, 0),
            RunOutcome::Partial
        );
    }

    #[test]
    fn path_derivation_uses_stem_and_avoids_self_collisions() {
        let plain = ArtifactPaths::from_output("out/proxies.txt");
        assert_eq!(plain.proxies, PathBuf::from("out/proxies.txt"));
        assert_eq!(plain.harvest, PathBuf::from("out/proxies.jsonl"));
        assert_eq!(plain.summary, PathBuf::from("out/proxies.summary.json"));

        let jsonl = ArtifactPaths::from_output("out/foo.jsonl");
        assert_eq!(
            jsonl.harvest,
            PathBuf::from("out/foo.jsonl.provenance.jsonl")
        );
        assert_eq!(jsonl.summary, PathBuf::from("out/foo.summary.json"));

        let summary = ArtifactPaths::from_output("out/foo.summary.json");
        assert_eq!(summary.harvest, PathBuf::from("out/foo.summary.jsonl"));
        assert_eq!(
            summary.summary,
            PathBuf::from("out/foo.summary.summary.json")
        );
    }

    #[test]
    fn task_failure_cannot_masquerade_as_a_full_run() {
        let providers = [provider("alpha", 1, vec![])];

        assert_eq!(
            compute_run_outcome(&providers, 1, 1, 1),
            RunOutcome::Partial
        );
    }

    #[test]
    fn atomic_write_replaces_contents_without_a_temp_file() {
        let dir = std::env::temp_dir().join(format!("proxplore-runner-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("artifact.txt");
        write_atomic(&path, b"new").unwrap();
        write_atomic(&path, b"replacement").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"replacement");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn provider_batch_order_not_completion_order_decides_duplicate_source() {
        let alpha = ProxyRecord {
            source: "alpha",
            ..record()
        };
        let beta = ProxyRecord {
            source: "beta",
            ..record()
        };
        let first = dedupe_provider_batches(vec![(0, vec![alpha.clone()]), (1, vec![beta])]);
        let second = dedupe_provider_batches(vec![
            (
                1,
                vec![ProxyRecord {
                    source: "beta",
                    ..record()
                }],
            ),
            (0, vec![alpha]),
        ]);

        let bytes = |records: &[ProxyRecord]| {
            records
                .iter()
                .flat_map(|record| {
                    serde_json::to_vec(&serde_json::json!({
                        "proxy": record.url(), "source": record.source
                    }))
                    .unwrap()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(bytes(&first), bytes(&second));
        assert_eq!(first[0].source, "alpha");
    }

    #[test]
    fn dedupe_preserves_first_source_and_collapses_same_provider_duplicates() {
        let first = ProxyRecord {
            source: "alpha",
            ..record()
        };
        let cross_provider = ProxyRecord {
            source: "beta",
            ..record()
        };
        let within_provider = ProxyRecord {
            source: "gamma",
            host: "other.example".into(),
            ..record()
        };
        let records = dedupe([
            first.clone(),
            cross_provider,
            within_provider.clone(),
            within_provider,
            first.clone(),
        ]);

        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|item| item == &first));
        assert!(!records.iter().any(|item| item.source == "beta"));
    }

    #[test]
    fn dedupe_sorts_by_scheme_host_and_port() {
        let mut socks = record();
        socks.scheme = Scheme::Socks5;
        socks.host = "z.example".into();
        socks.port = 1080;
        let http_high = ProxyRecord {
            port: 9000,
            ..record()
        };
        let http_low = ProxyRecord {
            host: "a.example".into(),
            port: 80,
            ..record()
        };
        let same_host_low_port = ProxyRecord {
            port: 8080,
            ..record()
        };

        let records = dedupe([socks, http_high, same_host_low_port, http_low]);

        assert_eq!(
            records
                .into_iter()
                .map(|record| record.url())
                .collect::<Vec<_>>(),
            [
                "http://a.example:80",
                "http://proxy.example:8080",
                "http://proxy.example:9000",
                "socks5://z.example:1080"
            ]
        );
    }

    #[test]
    fn dedupe_keeps_authenticated_and_unauthenticated_proxies_distinct() {
        let unauthenticated = record();
        let authenticated = ProxyRecord {
            user: Some("alice".into()),
            pass: Some("secret".into()),
            ..record()
        };

        let records = dedupe([authenticated.clone(), unauthenticated.clone()]);
        assert_eq!(records[0], unauthenticated);
        assert_eq!(records[1], authenticated);
    }

    #[test]
    fn dedupe_orders_equal_users_by_password() {
        let z_password = ProxyRecord {
            user: Some("alice".into()),
            pass: Some("z-password".into()),
            ..record()
        };
        let a_password = ProxyRecord {
            user: Some("alice".into()),
            pass: Some("a-password".into()),
            ..record()
        };

        let records = dedupe([z_password, a_password]);

        assert_eq!(
            records
                .into_iter()
                .map(|record| record.pass.as_deref().unwrap().to_owned())
                .collect::<Vec<_>>(),
            ["a-password", "z-password"]
        );
    }

    #[test]
    fn page_cap_is_hit_at_the_boundary() {
        assert!(!page_cap_hit(0, 2));
        assert!(!page_cap_hit(1, 2));
        assert!(page_cap_hit(2, 2));
    }

    #[test]
    fn advance_errors_are_truncated_while_none_is_exhaustion() {
        let truncated = advance_result(Err("missing authoritative total".into()), "http p1");
        assert!(truncated.0.is_none());
        assert!(truncated.1);
        assert_eq!(
            truncated.2.as_deref(),
            Some("http p1: missing authoritative total — truncated, not exhausted")
        );
        let exhausted = advance_result(Ok(None), "http p1");
        assert!(exhausted.0.is_none());
        assert!(!exhausted.1);
        assert!(exhausted.2.is_none());
    }

    #[test]
    fn advance_panic_is_truncated_even_with_no_next_request() {
        let panicked = advance_panic_result("http p1");
        assert!(panicked.0.is_none());
        assert!(panicked.1);
        assert_eq!(panicked.2.as_deref(), Some("http p1: advance panicked"));
    }

    #[test]
    fn failed_provider_keeps_real_identity_in_summary_metadata() {
        let failed = ProviderRun::failed("beta", 1.5, "provider task failed: cancelled");
        let providers = [provider("alpha", 1, vec![]), failed];

        assert_eq!(providers[1].provider_id, "beta");
        assert_eq!(providers[1].ok_requests, 0);
        assert_eq!(providers[1].records, 0);
        assert_eq!(providers[1].errors, ["provider task failed: cancelled"]);
        assert_eq!(
            compute_run_outcome(&providers, 1, 1, 1),
            RunOutcome::Partial
        );
    }

    #[test]
    fn rename_failure_removes_temp_and_preserves_target() {
        let dir = std::env::temp_dir().join(format!("proxplore-rename-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("target");
        fs::create_dir(&path).unwrap();

        assert!(write_atomic(&path, b"replacement").is_err());
        assert!(path.is_dir());
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn outcome_exit_codes_are_distinct_for_cron_consumers() {
        assert_eq!(RunOutcome::Full.exit_code(), 0);
        assert_eq!(RunOutcome::Failed.exit_code(), 1);
        assert_eq!(RunOutcome::Partial.exit_code(), 2);
    }

    fn record() -> ProxyRecord {
        ProxyRecord {
            scheme: Scheme::Http,
            host: "proxy.example".into(),
            port: 8080,
            user: None,
            pass: None,
            source: "alpha",
        }
    }

    #[test]
    fn failed_run_preserves_existing_nonempty_artifacts_but_updates_summary() {
        let dir = std::env::temp_dir().join(format!("proxplore-failed-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let paths = ArtifactPaths::from_output(dir.join("foo.txt").to_str().unwrap());
        fs::write(&paths.proxies, "last-good").unwrap();
        fs::write(&paths.harvest, "last-good-jsonl").unwrap();

        write_artifacts(
            &paths,
            &[],
            &[provider("alpha", 0, vec![])],
            RunOutcome::Failed,
            10,
            0.5,
            0,
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&paths.proxies).unwrap(), "last-good");
        assert_eq!(
            fs::read_to_string(&paths.harvest).unwrap(),
            "last-good-jsonl"
        );
        let summary: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&paths.summary).unwrap()).unwrap();
        assert_eq!(summary["outcome"], "failed");
        assert_eq!(summary["exit_code"], 1);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_run_preserves_nonempty_harvest_when_proxies_are_empty() {
        let dir = std::env::temp_dir().join(format!("proxplore-harvest-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let paths = ArtifactPaths::from_output(dir.join("foo.txt").to_str().unwrap());
        fs::write(&paths.proxies, "").unwrap();
        let prior_harvest = "{\"proxy\":\"http://proxy.example:8080\",\"source\":\"alpha\"}\n";
        fs::write(&paths.harvest, prior_harvest).unwrap();

        let report = write_artifacts(&paths, &[], &[], RunOutcome::Failed, 10, 0.5, 0).unwrap();

        assert!(!report.proxies_preserved);
        assert!(report.harvest_preserved);

        assert_eq!(fs::read(&paths.proxies).unwrap(), b"");
        assert_eq!(fs::read_to_string(&paths.harvest).unwrap(), prior_harvest);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_run_creates_empty_outputs_when_there_is_nothing_to_preserve() {
        let dir = std::env::temp_dir().join(format!("proxplore-empty-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let paths = ArtifactPaths::from_output(dir.join("foo.txt").to_str().unwrap());

        write_artifacts(&paths, &[], &[], RunOutcome::Failed, 10, 0.5, 0).unwrap();

        assert_eq!(fs::read(&paths.proxies).unwrap(), b"");
        assert_eq!(fs::read(&paths.harvest).unwrap(), b"");
        assert!(paths.summary.is_file());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn successful_artifacts_include_provenance_and_per_provider_duration() {
        let dir = std::env::temp_dir().join(format!("proxplore-json-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let paths = ArtifactPaths::from_output(dir.join("foo.txt").to_str().unwrap());
        let mut provider_run = provider("alpha", 1, vec![]);
        provider_run.duration_secs = 1.25;

        write_artifacts(
            &paths,
            &[record()],
            &[provider_run],
            RunOutcome::Full,
            123,
            2.5,
            1,
        )
        .unwrap();

        let harvest: serde_json::Value = serde_json::from_str(
            fs::read_to_string(&paths.harvest)
                .unwrap()
                .lines()
                .next()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(harvest["source"], "alpha");
        assert_eq!(harvest["run_started_at"], 123);
        let summary: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&paths.summary).unwrap()).unwrap();
        assert_eq!(summary["providers"][0]["duration_secs"], 1.25);
        assert_eq!(summary["exit_code"], 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn summary_directory_failure_leaves_successful_data_writes_visible() {
        let dir =
            std::env::temp_dir().join(format!("proxplore-summary-fail-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let paths = ArtifactPaths::from_output(dir.join("foo.txt").to_str().unwrap());
        fs::create_dir(&paths.summary).unwrap();

        let result = write_artifacts(
            &paths,
            &[record()],
            &[provider("alpha", 1, vec![])],
            RunOutcome::Full,
            42,
            1.0,
            1,
        );

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(&paths.proxies).unwrap(),
            "http://proxy.example:8080\n"
        );
        let harvest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&paths.harvest).unwrap()).unwrap();
        assert_eq!(harvest["proxy"], "http://proxy.example:8080");
        assert_eq!(harvest["run_started_at"], 42);
        assert!(paths.summary.is_dir());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn data_write_failure_commits_failed_summary() {
        let dir = std::env::temp_dir().join(format!("proxplore-write-fail-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let paths = ArtifactPaths::from_output(dir.join("foo.txt").to_str().unwrap());
        fs::create_dir(&paths.proxies).unwrap();
        fs::write(
            &paths.summary,
            serde_json::json!({"started_at": 7, "outcome": "full", "exit_code": 0}).to_string(),
        )
        .unwrap();

        assert!(
            write_artifacts(
                &paths,
                &[record()],
                &[provider("alpha", 1, vec![])],
                RunOutcome::Full,
                42,
                1.0,
                1,
            )
            .is_err()
        );

        let summary: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&paths.summary).unwrap()).unwrap();
        assert_eq!(summary["started_at"], 42);
        assert_eq!(summary["outcome"], "failed");
        assert_eq!(summary["exit_code"], 1);
        fs::remove_dir_all(dir).unwrap();
    }
}
