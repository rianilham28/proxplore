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

use std::error::Error;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tokio::task::JoinSet;

use fetch::{FetchConfig, Fetcher};
use log::{error, info, warn};
use model::{Provider, ScrapeOutcome};

#[derive(Parser)]
#[command(name = "proxplore", version, about, long_about = None)]
struct Cli {
    /// Output file (default: proxies.txt)
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    log::set_level(cli.verbose);

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
        for want in &cli.providers {
            match registry.iter().find(|p| p.id() == *want) {
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

    info(
        "proxplore",
        format_args!("scraping {} provider(s)…", selected.len()),
    );
    let mut set = JoinSet::new();
    for provider in selected {
        let fetcher = fetcher.clone();
        set.spawn(async move { runner::scrape(provider, fetcher).await });
    }
    let mut outcomes: Vec<ScrapeOutcome> = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(o) => {
                info(
                    "proxplore",
                    format_args!(
                        "done {:<20} requests {}/{} proxies={}",
                        o.provider_id,
                        o.requests_ok,
                        o.requests_total,
                        o.proxies.len()
                    ),
                );
                outcomes.push(o);
            }
            Err(e) => error("proxplore", format_args!("provider task failed: {e}")),
        }
    }

    for o in &outcomes {
        // a tripped-host fan-out produces one chain-stop per page; echo a
        // few and count the rest — the fetcher already logged the cause once
        for err in o.errors.iter().take(3) {
            warn(o.provider_id, format_args!("  {err}"));
        }
        if let Some(more) = o.errors.len().checked_sub(3) {
            warn(
                o.provider_id,
                format_args!("  … +{more} further per-page errors (same cause)"),
            );
        }
    }
    let merged = runner::dedupe(outcomes.into_iter().flat_map(|o| o.proxies));
    info(
        "proxplore",
        format_args!("unique after cross-provider dedupe: {}", merged.len()),
    );

    let written = runner::write_proxies(&cli.output, &merged)?;
    let mut counts: [usize; 4] = [0; 4];
    for p in &merged {
        counts[p.scheme as usize] += 1;
    }
    info(
        "proxplore",
        format_args!(
            "wrote {written} proxies -> {}  [http={}  https={}  socks4={}  socks5={}]",
            cli.output, counts[0], counts[1], counts[2], counts[3]
        ),
    );
    if written == 0 {
        std::process::exit(1);
    }
    Ok(())
}
