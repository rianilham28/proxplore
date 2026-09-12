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

async fn drain(
    provider: &dyn Provider,
    fetcher: &Fetcher,
    seed: Request,
    seen: Arc<Mutex<HashSet<String>>>,
) -> (Vec<ProxyRecord>, Vec<String>, usize, usize) {
    let id = provider.id();
    let mut proxies = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut req = Some(seed);
    let (mut made, mut ok) = (0usize, 0usize);
    let deadline = Instant::now() + provider.time_budget();

    while let Some(r) = req {
        if made >= provider.max_requests() {
            errors.push(format!(
                "{}: stopped at max_requests={} — feed may be larger (truncated, not exhausted)",
                r.label,
                provider.max_requests()
            ));
            break;
        }
        if Instant::now() >= deadline {
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
        let Some(body) = fetcher.get(&r.url, &r.headers).await else {
            errors.push(format!("{label}: fetch failed — chain stopped"));
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
        for rec in &mut found {
            if rec.source.is_empty() {
                rec.source = id; // shared parsers don't know the provider
            }
        }
        proxies.extend(found);
        ok += 1;
        req = match catch_unwind(AssertUnwindSafe(|| provider.advance(&r, &body))) {
            Ok(next) => next,
            Err(_) => {
                errors.push(format!("{label}: advance panicked"));
                break;
            }
        };
        if let Some(next) = &req {
            let mut set = seen.lock().expect("seen mutex poisoned");
            if !set.insert(next.url.clone()) {
                req = None; // covered by a sibling chain (or circular advance)
            }
        }
    }
    (proxies, errors, ok, made)
}

pub async fn scrape(provider: Arc<dyn Provider>, fetcher: Arc<Fetcher>) -> ScrapeOutcome {
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
            return outcome;
        }
    };
    let seen = Arc::new(Mutex::new(
        seeds
            .iter()
            .map(|s| s.url.clone())
            .collect::<HashSet<String>>(),
    ));
    let chains = join_all(seeds.into_iter().map(|seed| {
        let (provider, fetcher, seen) = (provider.clone(), fetcher.clone(), seen.clone());
        async move { drain(provider.as_ref(), &fetcher, seed, seen).await }
    }))
    .await;
    for (proxies, errors, ok, made) in chains {
        outcome.proxies.extend(proxies);
        outcome.errors.extend(errors);
        outcome.requests_ok += ok;
        outcome.requests_total += made;
    }
    outcome
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
    });
    out
}

/// One URL per line, atomic (write temp + rename). Returns line count.
pub fn write_proxies(path: &str, proxies: &[ProxyRecord]) -> Result<usize, std::io::Error> {
    let p = PathBuf::from(path);
    let dir: &Path = p
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".proxies-{}.tmp", std::process::id()));
    let mut written = 0usize;
    {
        let mut fh = fs::File::create(&tmp)?;
        for rec in proxies {
            writeln!(fh, "{}", rec.url())?;
            written += 1;
        }
        fh.flush()?;
    }
    fs::rename(&tmp, &p)?;
    Ok(written)
}
