//! proxplore — harvest free HTTP/HTTPS/SOCKS4/SOCKS5 proxies from keyless
//! public sources into a deduplicated proxies.txt (`scheme://[user:pass@]
//! host:port`, one per line).
//!
//! Scope by design: keyless public feeds only (TXT/JSON APIs, HTML list
//! pages, GitHub raw feeds). Node-subscription formats (vmess/vless/trojan/
//! ss), account-gated providers, and credential-leak "free socks5" pools are
//! out. Liveness VERIFICATION is not here either — that is proxalyze's job;
//! proxplore presents exactly what every source publishes, exhausted.
//!
//! Fetch width is sized from the machine (cpus, free memory, fd limits —
//! soft raised toward hard); --concurrency overrides. --proxy-url (or
//! $PROXPLORE_PROXY_URL) routes traffic through a rotating gateway so
//! per-IP rate limits collapse; deep feeds truncate LOUDLY (partial data
//! kept) when neither rotation nor budget lets a chain finish.

mod capacity;
mod fetch;
mod log;
mod model;
mod normalize;
mod parse;
mod providers;
mod runner;

use std::collections::HashSet;
use std::error::Error;
use std::io::Write;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::Parser;
use futures::FutureExt;
use tokio::task::JoinHandle;

use fetch::{FetchConfig, Fetcher};
use log::{error, info, warn};
use model::Provider;

#[derive(Parser)]
#[command(name = "proxplore", version, about, long_about = None)]
struct Cli {
    /// Proxy output. Writes derived provenance and summary artifacts beside
    /// it; a provenance name that would collide gets a suffix instead.
    /// Harvest exit status: 0 full, 1 failed, 2 partial (usage and
    /// provider-selection errors also exit nonzero).
    #[arg(short, long, default_value = "proxies.txt")]
    output: String,

    /// Only these provider ids (comma-separated) — for testing sources one
    /// by one. Default: every registered provider.
    #[arg(long, value_delimiter = ',')]
    providers: Vec<String>,

    /// Print the provider registry and exit
    #[arg(long)]
    list_providers: bool,

    /// Route all scrape traffic through an upstream proxy, e.g.
    /// http://user:pass@rotating-gw:1338 — defeats per-IP 429 walls on deep
    /// feeds. Falls back to $PROXPLORE_PROXY_URL so the secret can stay off
    /// argv.
    #[arg(long, env = "PROXPLORE_PROXY_URL")]
    proxy_url: Option<String>,

    /// Parallel HTTP fetches (default: auto-sized from device capacity)
    #[arg(long)]
    concurrency: Option<usize>,

    /// Per-request read timeout seconds (default: 30)
    #[arg(long, default_value_t = 30.0)]
    timeout: f64,

    /// Connect timeout seconds (default: 10) — a host that silently drops
    /// SYNs fails here, not on the full read budget
    #[arg(long, default_value_t = 10.0)]
    connect_timeout: f64,

    /// Debug logging
    #[arg(short, long)]
    verbose: bool,

    /// Suppress info logs (--verbose takes precedence if both are set)
    #[arg(short, long)]
    quiet: bool,
}

/// Registry invariant: every provider id is non-empty and unique. The CLI
/// selects by id and every record is attributed by id, so a collision or a
/// blank id is a real bug. (Wiring a module but forgetting to register it is
/// caught at COMPILE time — a never-called `new()` trips `dead_code` — so no
/// runtime filesystem scan is needed, and none would be correct now that the
/// homogeneous GitHub feeds live in one table module.)
fn guard_registry_integrity(all: &[Arc<dyn Provider>]) {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for p in all {
        let id = p.id();
        if id.is_empty() {
            error(
                "proxplore",
                format_args!("provider with empty id registered"),
            );
            std::process::exit(2);
        }
        if !seen.insert(id) {
            error("proxplore", format_args!("duplicate provider id: {id}"));
            std::process::exit(2);
        }
    }
}

fn unique_provider_ids(ids: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    ids.iter()
        .map(String::as_str)
        .filter(|id| seen.insert(*id))
        .collect()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    log::set_level(cli.quiet, cli.verbose);

    let registry = providers::all();
    guard_registry_integrity(&registry);

    if cli.list_providers {
        let mut rows: Vec<&Arc<dyn Provider>> = registry.iter().collect();
        rows.sort_by_key(|p| p.id());
        for p in rows {
            // Rust ignores SIGPIPE, so a closed consumer (head/grep) surfaces
            // as a write error — exit cleanly like any unix filter, not panic
            if writeln!(
                std::io::stdout(),
                "{:<20} {:<26} {:<22} {}",
                p.id(),
                p.protocols(),
                p.refresh(),
                p.site()
            )
            .is_err()
            {
                return Ok(());
            }
        }
        return Ok(());
    }

    let selected: Vec<Arc<dyn Provider>> = if cli.providers.is_empty() {
        registry.clone()
    } else {
        let mut out = Vec::new();
        for want in unique_provider_ids(&cli.providers) {
            match registry.iter().find(|p| p.id() == want) {
                Some(p) => out.push(p.clone()),
                None => {
                    error("proxplore", format_args!("unknown provider id: {want}"));
                    let mut ids: Vec<_> = registry.iter().map(|p| p.id()).collect();
                    ids.sort();
                    error("proxplore", format_args!("available: {}", ids.join(", ")));
                    std::process::exit(2);
                }
            }
        }
        out
    };

    let caps = capacity::probe(); // also lifts our own fd soft limit
    if let Some(p) = &cli.proxy_url {
        // credentials never echo: show scheme://host:port only
        let shown = url::Url::parse(p)
            .map(|u| {
                format!(
                    "{}://{}{}",
                    u.scheme(),
                    u.host_str().unwrap_or("?"),
                    u.port().map(|x| format!(":{x}")).unwrap_or_default()
                )
            })
            .unwrap_or_else(|_| "http://<unparsed-proxy>".into());
        info(
            "proxplore",
            format_args!("routing scrape traffic through proxy {shown}"),
        );
    }
    let fetcher = Arc::new(Fetcher::new(FetchConfig {
        concurrency: cli.concurrency.unwrap_or(caps.fetch_concurrency),
        timeout: Duration::from_secs_f64(cli.timeout),
        connect_timeout: Duration::from_secs_f64(cli.connect_timeout),
        proxy_url: cli.proxy_url.clone(),
    })?);

    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let run_started = Instant::now();
    info(
        "proxplore",
        format_args!("scraping {} provider(s)…", selected.len()),
    );
    let handles: Vec<(&'static str, JoinHandle<_>, Instant)> = selected
        .into_iter()
        .map(|provider| {
            let provider_id = provider.id();
            let fetcher = fetcher.clone();
            let spawn_started = Instant::now();
            let handle = tokio::spawn(async move {
                let started = Instant::now();
                let result = AssertUnwindSafe(runner::scrape(provider, fetcher))
                    .catch_unwind()
                    .await;
                (provider_id, started.elapsed().as_secs_f64(), result)
            });
            (provider_id, handle, spawn_started)
        })
        .collect();
    let mut provider_runs = Vec::new();
    let mut record_batches = Vec::new();
    let mut task_failures = 0usize;
    for (provider_id, handle, spawn_started) in handles {
        match handle.await {
            Ok((_, duration, Ok((outcome, truncated)))) => {
                info(
                    "proxplore",
                    format_args!(
                        "done {:<20} requests {}/{} proxies={} ({duration:.1}s)",
                        provider_id,
                        outcome.requests_ok,
                        outcome.requests_total,
                        outcome.proxies.len()
                    ),
                );
                provider_runs.push(runner::ProviderRun::from_outcome(
                    &outcome, duration, truncated,
                ));
                record_batches.push((provider_runs.len() - 1, outcome.proxies));
            }
            Ok((_, duration, Err(panic))) => {
                task_failures += 1;
                let reason = panic_message(&panic);
                error(
                    provider_id,
                    format_args!("provider task panicked: {reason}"),
                );
                provider_runs.push(runner::ProviderRun::failed(provider_id, duration, &reason));
                record_batches.push((provider_runs.len() - 1, Vec::new()));
            }
            Err(e) => {
                task_failures += 1;
                error(provider_id, format_args!("provider task failed: {e}"));
                // Cancellation is observed only after earlier handles, so this
                // is a spawn-to-observation upper bound rather than execution time.
                provider_runs.push(runner::ProviderRun::failed(
                    provider_id,
                    spawn_started.elapsed().as_secs_f64(),
                    &e.to_string(),
                ));
                record_batches.push((provider_runs.len() - 1, Vec::new()));
            }
        }
    }

    provider_runs.sort_by_key(|provider| provider.provider_id);

    for provider in &provider_runs {
        // a tripped-host fan-out produces one chain-stop per page; echo a
        // few and count the rest — the fetcher already logged the cause once
        for err in provider.errors.iter().take(3) {
            warn(provider.provider_id, format_args!("  {err}"));
        }
        if let Some(more) = provider.errors.len().checked_sub(3) {
            warn(
                provider.provider_id,
                format_args!("  … +{more} further per-page errors (same cause)"),
            );
        }
    }
    let records_total: usize = record_batches
        .iter()
        .map(|(_, records)| records.len())
        .sum();
    let merged = runner::dedupe_provider_batches(record_batches);
    info(
        "proxplore",
        format_args!("unique after cross-provider dedupe: {}", merged.len()),
    );

    let outcome =
        runner::compute_run_outcome(&provider_runs, records_total, merged.len(), task_failures);
    let paths = runner::ArtifactPaths::from_output(&cli.output);
    let report = runner::write_artifacts(
        &paths,
        &merged,
        &provider_runs,
        outcome,
        started_at,
        run_started.elapsed().as_secs_f64(),
        records_total,
    )?;

    let mut counts: [usize; 4] = [0; 4];
    for proxy in &merged {
        counts[proxy.scheme as usize] += 1;
    }
    info(
        "proxplore",
        format_args!(
            "{} {} proxies -> {}  [http={}  https={}  socks4={}  socks5={}]",
            if report.proxies_preserved {
                "kept last-good"
            } else {
                "wrote"
            },
            merged.len(),
            cli.output,
            counts[0],
            counts[1],
            counts[2],
            counts[3]
        ),
    );
    if outcome == runner::RunOutcome::Failed {
        warn(
            "proxplore",
            format_args!(
                "harvest produced no records; proxies {}, provenance {}",
                if report.proxies_preserved {
                    "preserved existing"
                } else {
                    "wrote empty"
                },
                if report.harvest_preserved {
                    "preserved existing"
                } else {
                    "wrote empty"
                },
            ),
        );
    }
    std::process::exit(outcome.exit_code());
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic payload".into())
}

#[cfg(test)]
mod tests {

    use super::unique_provider_ids;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn provider_ids_deduplicate_preserving_first_occurrence() {
        let selected = ids(&["alpha", "beta", "alpha", "gamma", "beta"]);

        assert_eq!(unique_provider_ids(&selected), ["alpha", "beta", "gamma"]);
    }

    #[test]
    fn unique_provider_ids_pass_through() {
        let selected = ids(&["gamma", "alpha", "beta"]);

        assert_eq!(unique_provider_ids(&selected), ["gamma", "alpha", "beta"]);
    }
}
