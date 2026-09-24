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
//! * HTTP 429: Retry-After (clamped to 1–120s, otherwise 5s) + retry, up to
//!   `rate_limit_retries` (default 3) extra tries — then fail. 429 is an
//!   egress-identity problem: the real fix is --proxy-url (rotating exit IP),
//!   not patience against one IP.
//!
//! Backoff sleeps happen OUTSIDE both global and per-host concurrency permits,
//! so waiting on a throttled host never parks a slot.

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
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use tokio::sync::Semaphore;
use wreq::{Client, Proxy, Response};
use wreq_util::Emulation;

use crate::log::{debug, warn};

const RETRY_STATUSES: [u16; 8] = [0, 408, 425, 429, 500, 502, 503, 504]; // 0 = aborted transfer
const CONNECT_TRIP: u32 = 8; // consecutive connection failures before the host trips
const PER_HOST_CONCURRENCY: usize = 8; // leaves global capacity for independent hosts
// Proxy lists are plain text and should be far below 32 MiB. The cap is generous
// for feeds while preventing a hostile endpoint from exhausting the process.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const CACHE_HOST: &str = "raw.githubusercontent.com";
const CACHE_FILE: &str = "feed-cache.json";
const MIN_RETRY_AFTER: Duration = Duration::from_secs(1);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(120);
const RATE_LIMIT_FALLBACK: Duration = Duration::from_secs(5);
const CACHE_FLUSH_DELAY: Duration = Duration::from_millis(250);

struct CacheSlot {
    entries: Mutex<HashMap<String, CachedFeed>>,
    flush_pending: AtomicBool,
}

impl CacheSlot {
    fn new(path: &std::path::Path) -> Self {
        Self {
            entries: Mutex::new(read_cache(path).unwrap_or_default()),
            flush_pending: AtomicBool::new(false),
        }
    }
}

// Each configured path gets one slot, so provider Fetcher clones share both
// the lazy disk load and future in-process updates.
type CacheSlotCell = Arc<std::sync::OnceLock<Arc<CacheSlot>>>;
static CACHE_SLOTS: LazyLock<Mutex<HashMap<std::path::PathBuf, CacheSlotCell>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CACHE_IO: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Outcome of one attempt.
enum Attempt {
    Ok(String),
    /// retriable transport/connection/status problem, described
    Retry {
        problem: String,
        connection_failure: bool,
    },
    /// HTTP 429 — handled by get()'s Retry-After-aware loop
    RateLimited(Duration),
    /// permanent problem (non-retriable status) — already logged
    Permanent,
    /// cacheable host answered 304 without a usable entry; retry once plain
    PlainGet,
}

struct HostState {
    consec_conn_fails: u32,
    tripped: bool,
    in_flight: Arc<Semaphore>,
}

impl HostState {
    fn new() -> Self {
        Self {
            consec_conn_fails: 0,
            tripped: false,
            in_flight: Arc::new(Semaphore::new(PER_HOST_CONCURRENCY)),
        }
    }
}

impl Default for HostState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]

struct CachedFeed {
    etag: String,
    body: String,
}

fn is_conn_failure(error: &wreq::Error) -> bool {
    error.is_connect()
        || error.is_proxy_connect()
        || error.is_timeout()
        || error.is_dns()
        || error.is_connection_reset()
}

fn header<'a>(
    headers: &'a wreq::header::HeaderMap,
    name: &wreq::header::HeaderName,
) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

fn declared_length_oversize(content_length: Option<&str>) -> bool {
    content_length
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
}

fn declared_body_oversize(response: &Response) -> bool {
    declared_length_oversize(header(response.headers(), &wreq::header::CONTENT_LENGTH))
}

enum BodyReadError {
    Oversize,
    Transport(wreq::Error),
}

async fn read_body(response: Response) -> Result<String, BodyReadError> {
    // Content-Length may describe compressed bytes, so the streaming decoded
    // body length below is the authoritative cap even when the header exists.
    let content_type = header(response.headers(), &wreq::header::CONTENT_TYPE)
        .and_then(|value| value.parse::<mime::Mime>().ok());
    let encoding_name = content_type
        .as_ref()
        .and_then(|mime| mime.get_param("charset").map(|charset| charset.as_str()))
        .unwrap_or("utf-8");
    let encoding =
        encoding_rs::Encoding::for_label(encoding_name.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    if declared_body_oversize(&response) {
        return Err(BodyReadError::Oversize);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(BodyReadError::Transport)?;
        if body.len().saturating_add(chunk.len()) > MAX_BODY_BYTES {
            return Err(BodyReadError::Oversize);
        }
        body.extend_from_slice(&chunk);
    }
    // Mirrors wreq 0.16.1 text_with_charset: Content-Type charset, UTF-8
    // default, unknown-label fallback, BOM sniffing, and lossy replacement.
    let (text, _, _) = encoding.decode(&body);
    Ok(text.into_owned())
}

fn parse_http_date(value: &str) -> Option<SystemTime> {
    // IMF-fixdate: Sun, 06 Nov 1994 08:49:37 GMT. Obsolete RFC850 and
    // asctime forms intentionally fall back to the 5s default.
    let mut fields = value.split_whitespace();

    let _weekday = fields.next()?;
    let day: u32 = fields.next()?.parse().ok()?;
    let month = match fields.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = fields.next()?.parse().ok()?;
    if !(1000..=9999).contains(&year) {
        return None;
    }
    let time: Vec<&str> = fields.next()?.split(':').collect();
    if fields.next()? != "GMT" || time.len() != 3 {
        return None;
    }
    let hour: i64 = time[0].parse().ok()?;
    let minute: i64 = time[1].parse().ok()?;
    let second: i64 = time[2].parse().ok()?;
    if !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }

    // Days from civil (Howard Hinnant's algorithm), shifted to 1970-01-01.
    let y = year.checked_sub(i64::from(month <= 2))?;
    let era = if y >= 0 { y } else { y.checked_sub(399)? } / 400;
    let yoe = y.checked_sub(era.checked_mul(400)?)?;
    let mp = month as i64 + if month > 2 { -3 } else { 9 };
    let doy = mp.checked_mul(153)?.checked_add(2)? / 5 + day as i64 - 1;
    let doe = yoe
        .checked_mul(365)?
        .checked_add(yoe / 4)?
        .checked_sub(yoe / 100)?
        .checked_add(doy)?;
    let days = era
        .checked_mul(146_097)?
        .checked_add(doe)?
        .checked_sub(719_468)?;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour.checked_mul(3_600)?)?
        .checked_add(minute.checked_mul(60)?)?
        .checked_add(second)?;
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))
    }
}

fn retry_after(value: Option<&str>, now: SystemTime) -> Duration {
    let parsed = value
        .and_then(|value| {
            value
                .trim()
                .parse::<u64>()
                .ok()
                .map(Duration::from_secs)
                .or_else(|| {
                    parse_http_date(value).map(|date| date.duration_since(now).unwrap_or_default())
                })
        })
        .unwrap_or(RATE_LIMIT_FALLBACK);
    parsed.clamp(MIN_RETRY_AFTER, MAX_RETRY_AFTER)
}

fn next_jitter_bits() -> u64 {
    // SystemTime seeds an atomic xorshift: enough entropy to desynchronize
    // sibling retries without a randomness crate or synchronized RNG state.
    static STATE: LazyLock<AtomicU64> = LazyLock::new(|| {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64);
        AtomicU64::new(seed | 1)
    });
    let mut current = STATE.load(Ordering::Relaxed);
    loop {
        let mut value = current;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        match STATE.compare_exchange_weak(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return value,
            Err(observed) => current = observed,
        }
    }
}

fn jittered(delay: Duration) -> Duration {
    // Preserve at least half the requested delay while spreading wakeups over
    // the other half; zero remains zero for deterministic immediate paths.
    delay.mul_f64(0.5 + 0.5 * (next_jitter_bits() >> 11) as f64 / ((1_u64 << 53) as f64))
}

fn server_retry_wait(server_delay: Duration) -> Duration {
    server_delay + jittered(server_delay / 2)
}

async fn sleep_bounded(wait: Duration, deadline: Option<Instant>, cancelled: &AtomicBool) -> bool {
    let started = Instant::now();
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return false;
        }
        let elapsed = started.elapsed();
        if elapsed >= wait {
            return deadline.is_none_or(|deadline| deadline > Instant::now());
        }
        let slice = (wait - elapsed).min(Duration::from_millis(250));
        let bounded = if let Some(deadline) = deadline {
            let remaining = deadline.checked_duration_since(Instant::now());
            match remaining {
                Some(remaining) if remaining.is_zero() => return false,
                Some(remaining) => slice.min(remaining),
                None => return false,
            }
        } else {
            slice
        };
        tokio::time::sleep(bounded).await;
    }
}

fn cache_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| {
        std::path::Path::new(&home)
            .join(".cache/proxplore")
            .join(CACHE_FILE)
    })
}

fn read_cache(path: &std::path::Path) -> Option<HashMap<String, CachedFeed>> {
    let data = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&data).ok()?;
    let entries = value.as_object()?;
    Some(
        entries
            .iter()
            .filter_map(|(url, value)| {
                let etag = value.get("etag")?.as_str()?;
                let body = value.get("body")?.as_str()?;
                if valid_etag(etag.trim()).is_none() || body.trim().is_empty() {
                    return None;
                }
                Some((
                    url.clone(),
                    CachedFeed {
                        etag: etag.trim().to_string(),

                        body: body.to_string(),
                    },
                ))
            })
            .collect(),
    )
}

fn write_cache_entries(
    path: &std::path::Path,
    entries: &HashMap<String, CachedFeed>,
) -> std::io::Result<()> {
    // Cache writes are best effort. Cross-process last-writer-wins may lose
    // one entry, which only makes the next run pay for a plain GET.
    let _guard = CACHE_IO
        .lock()
        .map_err(|_| std::io::Error::other("feed cache lock poisoned"))?;
    let value = serde_json::Value::Object(
        entries
            .iter()
            .map(|(url, cached)| {
                (
                    url.clone(),
                    serde_json::json!({ "etag": cached.etag, "body": cached.body }),
                )
            })
            .collect(),
    );
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("feed cache has no parent"))?;
    fs::create_dir_all(parent)?;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let data = serde_json::to_vec(&value).map_err(std::io::Error::other)?;
    if let Err(e) = fs::write(&temp, data) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    Ok(())
}

fn cache_slot(path: &std::path::Path) -> Arc<CacheSlot> {
    let slot_cell = {
        CACHE_SLOTS
            .lock()
            .expect("cache slot map poisoned")
            .entry(path.to_path_buf())
            .or_default()
            .clone()
    };
    Arc::clone(slot_cell.get_or_init(|| Arc::new(CacheSlot::new(path))))
}

fn cache_entry(slot: &CacheSlot, url: &str) -> Option<CachedFeed> {
    slot.entries
        .lock()
        .expect("feed cache poisoned")
        .get(url)
        .cloned()
}

fn update_cache(slot: &Arc<CacheSlot>, path: &std::path::Path, url: &str, etag: &str, body: &str) {
    let should_flush = {
        slot.entries.lock().expect("feed cache poisoned").insert(
            url.to_string(),
            CachedFeed {
                etag: etag.to_string(),
                body: body.to_string(),
            },
        );
        !slot.flush_pending.swap(true, Ordering::AcqRel)
    };
    if should_flush {
        schedule_cache_flush(Arc::clone(slot), path.to_path_buf());
    }
}

fn schedule_cache_flush(slot: Arc<CacheSlot>, path: std::path::PathBuf) {
    std::thread::spawn(move || {
        std::thread::sleep(CACHE_FLUSH_DELAY);
        if !slot.flush_pending.swap(false, Ordering::AcqRel) {
            return;
        }
        let entries = slot.entries.lock().expect("feed cache poisoned").clone();
        if let Err(e) = write_cache_entries(&path, &entries) {
            debug("fetch", format_args!("feed cache flush failed: {e}"));
        }
        if slot.flush_pending.load(Ordering::Acquire) {
            schedule_cache_flush(slot, path);
        }
    });
}

fn conditional_etag(entry: Option<&CachedFeed>) -> Option<&str> {
    entry
        .filter(|entry| !entry.body.trim().is_empty())
        .and_then(|entry| valid_etag(entry.etag.trim()))
}

fn valid_etag(etag: &str) -> Option<&str> {
    (!etag.is_empty()
        && etag
            .as_bytes()
            .iter()
            .all(|byte| (0x21..=0x7e).contains(byte)))
    .then_some(etag)
}

pub struct Fetcher {
    client: Client,
    proxy: Option<Proxy>,
    sem: Arc<Semaphore>,
    retries: usize,
    rate_limit_retries: usize,
    hosts: Mutex<HashMap<String, HostState>>,
    cache_host: String,
    cache_file: Option<std::path::PathBuf>,
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
            hosts: Mutex::new(HashMap::new()),
            cache_host: CACHE_HOST.to_string(),
            cache_file: cache_path(),
        })
    }

    fn host_of(url: &str) -> String {
        url::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| url.to_string())
    }

    fn host_permit(&self, host: &str) -> Arc<Semaphore> {
        self.hosts
            .lock()
            .expect("host map poisoned")
            .entry(host.to_string())
            .or_default()
            .in_flight
            .clone()
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

    async fn send(
        &self,
        url: &str,
        headers: &[(String, String)],
        etag: Option<&str>,
    ) -> Result<Response, Attempt> {
        let mut req = self.client.get(url).emulation(Emulation::Chrome149);
        if let Some(p) = &self.proxy {
            req = req.proxy(p.clone());
        }
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some(etag) = etag {
            req = req.header("If-None-Match", etag);
        }
        req.send().await.map_err(|e| Attempt::Retry {
            problem: format!("{e:?}"),
            connection_failure: is_conn_failure(&e),
        })
    }

    async fn handle_response(
        &self,
        url: &str,
        host: &str,
        cache: Option<(&Arc<CacheSlot>, &std::path::Path)>,
        cached_entry: Option<CachedFeed>,
        allow_plain_fallback: bool,
        resp: Response,
    ) -> Attempt {
        let status = resp.status().as_u16();
        if status == 304 && host == self.cache_host {
            if let Some(body) = cached_entry.map(|entry| entry.body) {
                return Attempt::Ok(body);
            }
            if allow_plain_fallback {
                return Attempt::PlainGet;
            }
            warn(
                "fetch",
                format_args!("{url} -> HTTP 304 on plain GET (skipped)"),
            );
            return Attempt::Permanent;
        }

        if status == 200 {
            let etag = header(resp.headers(), &wreq::header::ETAG).map(str::to_string);
            return match read_body(resp).await {
                Ok(body) if !body.trim().is_empty() => {
                    if let (Some((slot, path)), Some(etag)) = (cache, etag) {
                        update_cache(slot, path, url, &etag, &body);
                    }
                    Attempt::Ok(body)
                }
                Ok(_) => Attempt::Retry {
                    problem: "200 but empty body".into(),
                    connection_failure: false,
                },
                Err(BodyReadError::Oversize) => Attempt::Retry {
                    problem: format!("response body exceeds {MAX_BODY_BYTES} bytes"),
                    connection_failure: false,
                },
                Err(BodyReadError::Transport(e)) => Attempt::Retry {
                    problem: format!("body read failed: {e}"),
                    connection_failure: is_conn_failure(&e),
                },
            };
        }
        if status == 429 {
            return Attempt::RateLimited(retry_after(
                header(resp.headers(), &wreq::header::RETRY_AFTER),
                SystemTime::now(),
            ));
        }
        if RETRY_STATUSES.contains(&status) {
            return Attempt::Retry {
                problem: format!("HTTP {status}"),
                connection_failure: false,
            };
        }
        warn("fetch", format_args!("{url} -> HTTP {status} (skipped)"));
        Attempt::Permanent
    }

    async fn once(&self, url: &str, host: &str, headers: &[(String, String)]) -> Attempt {
        let cached = if host == self.cache_host {
            self.cache_file
                .as_deref()
                .map(|path| (path, cache_slot(path)))
        } else {
            None
        };
        let cached_entry = cached.as_ref().and_then(|(_, slot)| cache_entry(slot, url));
        // Global admission happens first; once admitted, a host waiting for
        // one of its own eight slots temporarily holds a global slot.
        let _global_permit = self.sem.acquire().await.expect("semaphore closed");
        let _host_permit = self
            .host_permit(host)
            .acquire_owned()
            .await
            .expect("host semaphore closed");
        let path_slot = cached.as_ref().map(|(path, slot)| (slot, *path));
        match self
            .send(url, headers, conditional_etag(cached_entry.as_ref()))
            .await
        {
            Ok(resp) => {
                match self
                    .handle_response(url, host, path_slot, cached_entry, true, resp)
                    .await
                {
                    Attempt::PlainGet => match self.send(url, headers, None).await {
                        Ok(resp) => {
                            self.handle_response(url, host, path_slot, None, false, resp)
                                .await
                        }
                        Err(attempt) => attempt,
                    },
                    attempt => attempt,
                }
            }
            Err(attempt) => attempt,
        }
    }

    /// Returns body text, or None on failure (already logged).
    pub async fn get(
        &self,
        url: &str,
        headers: &[(String, String)],
        deadline: Option<Instant>,
        cancelled: &AtomicBool,
    ) -> Option<String> {
        if cancelled.load(Ordering::Relaxed) {
            debug(
                "fetch",
                format_args!("{url}: fetch cancelled before request"),
            );
            return None;
        }
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
            if cancelled.load(Ordering::Relaxed) {
                problem = "fetch cancelled".into();
                break;
            }
            if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
                problem = "provider deadline reached".into();
                break;
            }
            attempt += 1;
            match self.once(url, &host, headers).await {
                Attempt::Ok(body) => {
                    self.note_success(&host);
                    return Some(body);
                }
                Attempt::Permanent => return None,
                Attempt::PlainGet => {
                    problem = "HTTP 304 without usable cache entry".into();
                    if self.is_tripped(&host) {
                        problem += " (circuit opened)";
                        break;
                    }
                }

                Attempt::RateLimited(server_delay) => {
                    rate_hits += 1;
                    if rate_hits > self.rate_limit_retries {
                        problem = "HTTP 429".into();
                        break;
                    }
                    let wait = server_retry_wait(server_delay);
                    debug(
                        "fetch",
                        format_args!("{url} rate-limited, waiting up to {wait:?}"),
                    );
                    if !sleep_bounded(wait, deadline, cancelled).await {
                        problem = if cancelled.load(Ordering::Relaxed) {
                            "HTTP 429; fetch cancelled".into()
                        } else {
                            "HTTP 429; provider deadline reached".into()
                        };
                        break;
                    }
                    continue;
                }
                Attempt::Retry {
                    problem: detail,
                    connection_failure,
                } => {
                    problem = detail;
                    if connection_failure {
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
            let wait = jittered(Duration::from_secs_f64(1.5 * attempt as f64));
            if !sleep_bounded(wait, deadline, cancelled).await {
                problem.push_str(if cancelled.load(Ordering::Relaxed) {
                    "; fetch cancelled"
                } else {
                    "; provider deadline reached"
                });
                break;
            }
        }
        warn(
            "fetch",
            format_args!("{url} failed after {attempt} attempts: {problem}"),
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{Barrier, oneshot};

    fn test_fetcher() -> Fetcher {
        let mut fetcher = Fetcher::new(FetchConfig {
            concurrency: 32,
            timeout: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(2),
            proxy_url: None,
        })
        .unwrap();
        fetcher.retries = 0;
        fetcher.rate_limit_retries = 0;
        fetcher
    }

    fn test_cache_path(prefix: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "proxplore-fetch-{prefix}-{}-{}",
            std::process::id(),
            next_jitter_bits()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CACHE_FILE);
        (dir, path)
    }

    async fn read_request_headers(reader: &mut BufReader<TcpStream>) -> std::io::Result<String> {
        let mut request = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await? == 0 || line == "\r\n" {
                return Ok(request);
            }
            request.push_str(&line);
        }
    }

    async fn write_response(
        stream: &mut TcpStream,
        status: &str,
        body: &str,
    ) -> std::io::Result<()> {
        stream
            .write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
    }

    async fn write_response_raw(stream: &mut TcpStream, response: &str) -> std::io::Result<()> {
        stream.write_all(response.as_bytes()).await
    }

    #[test]
    fn retry_after_seconds_are_clamped() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert_eq!(retry_after(Some("7"), now), Duration::from_secs(7));
        assert_eq!(retry_after(Some("0"), now), MIN_RETRY_AFTER);
        assert_eq!(retry_after(Some("999"), now), MAX_RETRY_AFTER);
        assert_eq!(retry_after(Some("nonsense"), now), RATE_LIMIT_FALLBACK);
        assert_eq!(retry_after(None, now), RATE_LIMIT_FALLBACK);
    }

    #[test]
    fn retry_after_http_date_is_clamped() {
        let date = "Wed, 21 Oct 2015 07:28:00 GMT";
        let date_seconds = 1_445_412_480;
        assert_eq!(
            parse_http_date(date),
            Some(UNIX_EPOCH + Duration::from_secs(date_seconds))
        );
        assert_eq!(
            retry_after(
                Some(date),
                UNIX_EPOCH + Duration::from_secs(date_seconds - 45)
            ),
            Duration::from_secs(45)
        );
        assert_eq!(
            retry_after(
                Some(date),
                UNIX_EPOCH + Duration::from_secs(date_seconds + 3_600)
            ),
            MIN_RETRY_AFTER
        );
        assert_eq!(
            retry_after(
                Some("Sun, 06 Nov 1994 08:49:37 GMT"),
                UNIX_EPOCH + Duration::from_secs(784_111_777)
            ),
            MIN_RETRY_AFTER
        );
        assert_eq!(
            retry_after(Some("Wed, 21 Oct 2099 07:28:00 GMT"), UNIX_EPOCH),
            MAX_RETRY_AFTER
        );
        assert_eq!(
            retry_after(Some("not an HTTP date"), UNIX_EPOCH),
            RATE_LIMIT_FALLBACK
        );
        assert_eq!(
            retry_after(Some("Sun, 06 Nov 999999999 08:49:37 GMT"), UNIX_EPOCH),
            RATE_LIMIT_FALLBACK
        );
    }

    #[test]
    fn jitter_stays_within_requested_half_range() {
        for delay in [
            Duration::from_secs(1),
            Duration::from_secs(5),
            Duration::from_secs(120),
        ] {
            for _ in 0..1_000 {
                let actual = jittered(delay);
                assert!(actual >= delay / 2);
                assert!(actual <= delay);
            }
        }
    }

    #[test]
    fn server_retry_jitter_preserves_server_floor() {
        let delay = Duration::from_secs(60);
        for _ in 0..1_000 {
            let actual = server_retry_wait(delay);
            assert!(actual >= delay);
            assert!(actual <= delay + delay / 2);
        }
    }

    #[tokio::test]
    async fn retry_sleep_is_bounded_cancellable_and_past_deadline_is_immediate() {
        let started = Instant::now();
        let cancelled = AtomicBool::new(false);
        assert!(!sleep_bounded(Duration::from_secs(60), Some(Instant::now()), &cancelled).await);
        assert!(started.elapsed() < Duration::from_millis(100));

        let deadline = Instant::now() + Duration::from_millis(20);
        assert!(!sleep_bounded(Duration::from_secs(60), Some(deadline), &cancelled).await);
        assert!(Instant::now() >= deadline);

        let cancelled = AtomicBool::new(true);
        assert!(!sleep_bounded(Duration::from_secs(60), None, &cancelled).await);
        assert!(Instant::now() < started + Duration::from_millis(500));
    }

    #[test]
    fn declared_length_decision_rejects_only_oversize() {
        assert!(!declared_length_oversize(Some(
            &(MAX_BODY_BYTES as u64).to_string()
        )));
        assert!(declared_length_oversize(Some(
            &(MAX_BODY_BYTES as u64 + 1).to_string()
        )));
        assert!(!declared_length_oversize(None));
        assert!(!declared_length_oversize(Some("garbage")));
    }

    #[test]
    fn cache_roundtrip_preserves_etag_and_body() {
        let dir = std::env::temp_dir().join(format!(
            "proxplore-fetch-test-{}-{}",
            std::process::id(),
            next_jitter_bits()
        ));
        let path = dir.join(CACHE_FILE);
        let url = "https://raw.githubusercontent.com/example/main/proxies.txt";
        let cached = CachedFeed {
            etag: "\"abc\"".into(),
            body: "1.2.3.4:80".into(),
        };
        let mut entries = HashMap::new();
        entries.insert(url.to_string(), cached.clone());
        write_cache_entries(&path, &entries).unwrap();
        assert_eq!(read_cache(&path).unwrap().get(url), Some(&cached));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn failed_cache_rename_removes_temp_file() {
        let dir = std::env::temp_dir().join(format!(
            "proxplore-cache-rename-{}-{}",
            std::process::id(),
            next_jitter_bits()
        ));
        let target = dir.join(CACHE_FILE);
        fs::create_dir_all(&target).unwrap();
        let result = write_cache_entries(
            &target,
            &HashMap::from([(
                "https://example.test/feed".to_string(),
                CachedFeed {
                    etag: "\"abc\"".into(),
                    body: "1.2.3.4:80".into(),
                },
            )]),
        );
        assert!(result.is_err());
        let temp = target.with_extension(format!("tmp-{}", std::process::id()));
        assert!(!temp.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cache_lookups_continue_after_file_deletion() {
        let (dir, path) = test_cache_path("memory");
        let url = "https://raw.githubusercontent.com/example/main/proxies.txt";
        let cached = CachedFeed {
            etag: "\"cached-v1\"".into(),
            body: "1.2.3.4:8080".into(),
        };
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                url: { "etag": cached.etag, "body": cached.body }
            }))
            .unwrap(),
        )
        .unwrap();
        let slot = cache_slot(&path);
        assert_eq!(cache_entry(&slot, url), Some(cached));
        fs::remove_file(&path).unwrap();
        assert_eq!(
            cache_entry(&slot, url),
            Some(CachedFeed {
                etag: "\"cached-v1\"".into(),
                body: "1.2.3.4:8080".into(),
            })
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn valid_cache_entry_conditions_and_serves_304_body() {
        let entry = CachedFeed {
            etag: " \"abc\" ".into(),
            body: "1.2.3.4:80".into(),
        };
        assert_eq!(conditional_etag(Some(&entry)), Some("\"abc\""));
        let served = Some(entry.clone())
            .map(|entry| entry.body)
            .filter(|body| !body.trim().is_empty())
            .expect("304 serves valid cached body");
        assert_eq!(served, "1.2.3.4:80");
    }

    #[test]
    fn cache_rejects_unusable_entries() {
        let dir = std::env::temp_dir().join(format!(
            "proxplore-fetch-invalid-{}-{}",
            std::process::id(),
            next_jitter_bits()
        ));
        let path = dir.join(CACHE_FILE);
        let url = "https://raw.githubusercontent.com/example/main/proxies.txt";
        fs::create_dir_all(&dir).unwrap();
        for (etag, body) in [
            ("", "1.2.3.4:80"),
            (" \n\t", "1.2.3.4:80"),
            ("\"abc\"", " \n\t"),
            ("a b", "1.2.3.4:80"),
            ("a\u{7f}", "1.2.3.4:80"),
            ("aé", "1.2.3.4:80"),
            ("\0", "1.2.3.4:80"),
        ] {
            fs::write(
                &path,
                serde_json::to_vec(&serde_json::json!({ url: { "etag": etag, "body": body } }))
                    .unwrap(),
            )
            .unwrap();
            let loaded = read_cache(&path).unwrap_or_default();
            assert!(!loaded.contains_key(url));
            assert!(conditional_etag(loaded.get(url)).is_none());
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn chunked_body_over_limit_is_rejected() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n",
                )
                .await;
            let size = format!("{:x}\r\n", MAX_BODY_BYTES + 1);
            let _ = stream.write_all(size.as_bytes()).await;

            let _ = stream.write_all(&vec![b'x'; MAX_BODY_BYTES + 1]).await;
            let _ = stream.write_all(b"\r\n0\r\n\r\n").await;
        });

        let cancelled = AtomicBool::new(false);
        let body = test_fetcher()
            .get(&format!("http://{address}/large"), &[], None, &cancelled)
            .await;

        assert_eq!(body, None);
        let _ = server.await;
    }

    #[tokio::test]
    async fn conditional_get_serves_seeded_cache_on_not_modified() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}/proxies.txt");
        let (request_sender, request_receiver) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let request = read_request_headers(&mut reader).await.unwrap();
            let _ = request_sender.send(request);
            write_response_raw(
                reader.get_mut(),
                "HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        });
        let (dir, path) = test_cache_path("conditional");
        let cached = CachedFeed {
            etag: "\"seeded-v1\"".into(),
            body: "1.2.3.4:8080".into(),
        };
        let cache_host = url::Url::parse(&url)
            .unwrap()
            .host_str()
            .unwrap()
            .to_string();
        let mut entries = serde_json::Map::new();
        entries.insert(
            url.clone(),
            serde_json::json!({ "etag": &cached.etag, "body": &cached.body }),
        );
        fs::write(&path, serde_json::to_vec(&entries).unwrap()).unwrap();

        let mut fetcher = test_fetcher();
        fetcher.cache_host = cache_host;
        fetcher.cache_file = Some(path);

        let body = fetcher.get(&url, &[], None, &AtomicBool::new(false)).await;
        let request = request_receiver.await.unwrap();
        server.await.unwrap();

        let request = request.to_ascii_lowercase();
        assert!(request.contains("if-none-match: \"seeded-v1\"\r\n"));
        assert_eq!(body.as_deref(), Some(cached.body.as_str()));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn fetcher_limits_concurrency_and_restores_permits() {
        let request_count = PER_HOST_CONCURRENCY * 2;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let current = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let first_wave = Arc::new(Barrier::new(PER_HOST_CONCURRENCY));
        let server_current = Arc::clone(&current);
        let server_maximum = Arc::clone(&maximum);
        let server = tokio::spawn(async move {
            let mut connections = Vec::new();
            for _ in 0..request_count {
                let (stream, _) = listener.accept().await.unwrap();
                let current = Arc::clone(&server_current);
                let maximum = Arc::clone(&server_maximum);
                let first_wave = Arc::clone(&first_wave);
                connections.push(tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let request = read_request_headers(&mut reader).await.unwrap();
                    let active = current.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(active, Ordering::SeqCst);
                    first_wave.wait().await;
                    current.fetch_sub(1, Ordering::SeqCst);
                    if request.starts_with("GET /error/") {
                        write_response(reader.get_mut(), "400 Bad Request", "")
                            .await
                            .unwrap();
                    } else {
                        write_response(reader.get_mut(), "200 OK", "ok")
                            .await
                            .unwrap();
                    }
                }));
            }
            for connection in connections {
                connection.await.unwrap();
            }
        });

        let fetcher = Arc::new(test_fetcher());
        let requests = (0..request_count).map(|index| {
            let fetcher = Arc::clone(&fetcher);
            let url = if index % 2 == 0 {
                format!("http://{address}/ok/{index}")
            } else {
                format!("http://{address}/error/{index}")
            };
            tokio::spawn(async move { fetcher.get(&url, &[], None, &AtomicBool::new(false)).await })
        });
        let results = futures::future::join_all(requests)
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        server.await.unwrap();

        assert_eq!(maximum.load(Ordering::SeqCst), PER_HOST_CONCURRENCY);
        for (index, result) in results.into_iter().enumerate() {
            if index % 2 == 0 {
                assert_eq!(result.as_deref(), Some("ok"));
            } else {
                assert_eq!(result, None);
            }
        }
    }
}
