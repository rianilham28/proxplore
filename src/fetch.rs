//! Transport component: async wreq fetcher.
//!
//! Chrome TLS/H2 fingerprinting via request-level `.emulation()` is what
//! unlocks sources that 403 plain clients (GeoNode and friends) without a
//! headless browser. Retry policy, optional upstream (rotating) proxy, and
//! concurrency live here so providers stay purely declarative.
//!
//! Retry policy (deliberately simple, matching the proven Python behavior):
//! * transport errors / transient statuses: `retries` attempts
//!   (default 2), short 1.5s ladder;
//! * HTTP 429: a constant 5s wait + retry, up to `rate_limit_retries`
//!   (default 3) extra tries — then fail. 429 is an egress-identity problem:
//!   the real fix is --proxy-url (rotating exit IP), not patience against one
//!   IP.
//!
//! Backoff sleeps happen OUTSIDE the concurrency permit, so waiting on a
//! throttled host never parks a slot.
//!
//! Per-host circuit breaker: a host that blackholes us (SYN dropped —
//! e.g. proxydb.net against a residential IP after heavy direct probing)
//! makes every attempt burn the full connect timeout. After `CONNECT_TRIP`
//! consecutive connection failures the host trips: further requests fail
//! instantly for the rest of the run (one loud warning at trip time, debug
//! lines after). The trip is re-checked after EVERY failure, so a fan-out of
//! sibling chains stops within one attempt of the breaker firing. Successes
//! reset the counter; a trip itself is sticky — a host that black-holed a
//! whole fan-out will not reconsider mid-run.
//!
//! Rotating-proxy mode: gateways such as plainproxies rotate the egress IP
//! per NEW TCP/CONNECT connection (a reused keep-alive tunnel pins one IP,
//! and a `Connection: close` header does not force a fresh CONNECT tunnel).
//! With `proxy=` set, every attempt runs through a fresh client connection
//! (pool_idle 0) — fresh handshake, fresh exit, even across retries.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Semaphore;
use wreq::{Client, Proxy};
use wreq_util::Emulation;

use crate::log::{debug, warn};

const RETRY_STATUSES: [u16; 8] = [0, 408, 425, 429, 500, 502, 503, 504]; // 0 = aborted transfer
const CONNECT_TRIP: u32 = 8; // consecutive connection failures before the host trips

/// Outcome of one attempt.
enum Attempt {
    Ok(String),
    /// retriable transport/connection/status problem, described
    Retry(String),
    /// HTTP 429 — handled by get()'s constant-wait loop
    RateLimited,
    /// permanent problem (non-retriable status) — already logged
    Permanent,
}

#[derive(Default)]
struct HostState {
    consec_conn_fails: u32,
    tripped: bool,
}

fn is_conn_failure(msg: &str) -> bool {
    msg.contains("Connect") || msg.contains("TimedOut") || msg.contains("timed out")
}

pub struct Fetcher {
    client: Client,
    proxy: Option<Proxy>,
    sem: Arc<Semaphore>,
    retries: usize,
    rate_limit_retries: usize,
    rate_limit_wait: Duration,
    hosts: Mutex<HashMap<String, HostState>>,
}

pub struct FetchConfig {
    pub concurrency: usize,
    pub timeout: Duration,
    pub connect_timeout: Duration,
    pub proxy_url: Option<String>,
}

impl Fetcher {
    pub fn new(cfg: FetchConfig) -> Result<Self, Box<dyn std::error::Error>> {
        let proxy = match &cfg.proxy_url {
            Some(p) => Some(Proxy::all(p.as_str())?),
            None => None,
        };
        let mut b = Client::builder()
            .timeout(cfg.timeout)
            .connect_timeout(cfg.connect_timeout)
            // requests/curl_cffi follow redirects by default; wreq does not
            // unless a policy is set — without it CN hosts 301 and we skip
            .redirect(wreq::redirect::Policy::default()) // limited(10)
            .gzip(true)
            .brotli(true)
            .zstd(true);
        if proxy.is_some() {
            // fresh connection per request = fresh egress IP (see module doc)
            b = b.pool_max_idle_per_host(0);
        }
        Ok(Fetcher {
            client: b.build()?,
            proxy,
            sem: Arc::new(Semaphore::new(cfg.concurrency)),
            retries: 2,
            rate_limit_retries: 3,
            rate_limit_wait: Duration::from_secs(5),
            hosts: Mutex::new(HashMap::new()),
        })
    }

    fn host_of(url: &str) -> String {
        url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| url.to_string())
    }

    fn is_tripped(&self, host: &str) -> bool {
        self.hosts
            .lock()
            .expect("host map poisoned")
            .get(host)
            .is_some_and(|s| s.tripped)
    }

    /// Records one connection failure; trips (and warns once) at the limit.
    fn note_conn_fail(&self, host: &str, sample: &str) {
        let trip_now = {
            let mut guard = self.hosts.lock().expect("host map poisoned");
            let st = guard.entry(host.to_string()).or_default();
            st.consec_conn_fails += 1;
            if st.consec_conn_fails == CONNECT_TRIP && !st.tripped {
                st.tripped = true;
                true
            } else {
                false
            }
        };
        if trip_now {
            warn(
                "fetch",
                format_args!(
                    "{host}: {CONNECT_TRIP} consecutive connection failures — circuit open, \
                     failing fast for the rest of the run (this host blackholes our egress; \
                     a rotating --proxy-url restores it). last error: {sample}"
                ),
            );
        }
    }

    fn note_success(&self, host: &str) {
        if let Some(st) = self.hosts.lock().expect("host map poisoned").get_mut(host) {
            st.consec_conn_fails = 0;
        }
    }

    async fn once(&self, url: &str, headers: &[(String, String)]) -> Attempt {
        let _permit = self.sem.acquire().await.expect("semaphore closed");
        let mut req = self.client.get(url).emulation(Emulation::Chrome149);
        if let Some(p) = &self.proxy {
            req = req.proxy(p.clone());
        }
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => return Attempt::Retry(format!("{e:?}")),
        };
        let status = resp.status().as_u16();
        if status == 200 {
            return match resp.text().await {
                Ok(t) if !t.trim().is_empty() => Attempt::Ok(t),
                Ok(_) => Attempt::Retry("200 but empty body".into()),
                Err(e) => Attempt::Retry(format!("body read failed: {e}")),
            };
        }
        if status == 429 {
            return Attempt::RateLimited;
        }
        if RETRY_STATUSES.contains(&status) {
            return Attempt::Retry(format!("HTTP {status}"));
        }
        warn("fetch", format_args!("{url} -> HTTP {status} (skipped)"));
        Attempt::Permanent
    }

    /// Returns body text, or None on failure (already logged).
    pub async fn get(&self, url: &str, headers: &[(String, String)]) -> Option<String> {
        let host = Self::host_of(url);
        if self.is_tripped(&host) {
            debug(
                "fetch",
                format_args!("{url}: host circuit open, instant fail"),
            );
            return None;
        }
        let mut attempt = 0usize;
        let mut rate_hits = 0usize;
        let mut problem: String;
        loop {
            attempt += 1;
            match self.once(url, headers).await {
                Attempt::Ok(body) => {
                    self.note_success(&host);
                    return Some(body);
                }
                Attempt::Permanent => return None,
                Attempt::RateLimited => {
                    rate_hits += 1;
                    if rate_hits > self.rate_limit_retries {
                        problem = "HTTP 429".into();
                        break;
                    }
                    debug(
                        "fetch",
                        format_args!("{url} rate-limited, waiting {:?}", self.rate_limit_wait),
                    );
                    tokio::time::sleep(self.rate_limit_wait).await; // outside the permit
                    continue;
                }
                Attempt::Retry(p) => {
                    problem = p;
                    if is_conn_failure(&problem) {
                        self.note_conn_fail(&host, &problem);
                    }
                    // checked after every failure: a sibling chain may have
                    // tripped this host while the attempt was in flight
                    if self.is_tripped(&host) {
                        problem += " (circuit opened)";
                        break;
                    }
                }
            }
            if attempt > self.retries {
                break;
            }
            tokio::time::sleep(Duration::from_secs_f64(1.5 * attempt as f64)).await;
        }
        warn(
            "fetch",
            format_args!("{url} failed after {attempt} attempts: {problem}"),
        );
        None
    }
}
