//! The contract: every provider and component speaks these types.
//!
//! Port of the proven Python contract (proxplore/contract.py): a provider is
//! an *identified* source that declares `requests()` — a seed plan of
//! [`Request`]s. Parsing dispatches per request through [`ParseKind`]:
//! a shared component (Entries/Json/Ndjson/Auto) or a provider-local `fn`
//! pointer (bespoke one-source quirks live in the provider module, never
//! here). Paged feeds grow through [`Provider::advance`] so each source is
//! harvested to exhaustion; the drain loop in `runner` enforces caps
//! (max requests, time budget, a shared seen-set that also kills loops).
//!
//! Object safety note: the trait has no async methods — the async pipeline
//! lives in `runner::scrape`, keeping the whole design dependency-free
//! (no async-trait).

use std::time::Duration;

/// Transport a proxy speaks. Sources spell protocols every which way;
/// [`Scheme::from_label`] canonicalizes them (socks4a→socks4, ssl→https...).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Scheme {
    Http,
    Https,
    Socks4,
    Socks5,
}

impl Scheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
            Scheme::Socks4 => "socks4",
            Scheme::Socks5 => "socks5",
        }
    }

    /// Map a source's protocol spelling to a canonical scheme.
    pub fn from_label(raw: &str) -> Option<Scheme> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "http" => Some(Scheme::Http),
            "https" | "ssl" => Some(Scheme::Https),
            "socks4" | "socks4a" => Some(Scheme::Socks4),
            "socks" | "socks5" | "socks5h" => Some(Scheme::Socks5),
            _ => None,
        }
    }
}

impl std::fmt::Display for Scheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One normalized proxy. `url()` is the deliverable format:
/// `scheme://[user:pass@]host:port`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyRecord {
    pub scheme: Scheme,
    /// IPv4 dotted, bracketed IPv6 (`[2a01::1]`), or lowercase hostname.
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
    /// PROVIDER_ID that yielded this record (attribution survives dedupe).
    pub source: &'static str,
}
impl ProxyRecord {
    pub fn url(&self) -> String {
        match &self.user {
            Some(u) => format!(
                "{}://{}:{}@{}:{}",
                self.scheme,
                u,
                self.pass.as_deref().unwrap_or(""),
                self.host,
                self.port
            ),
            None => format!("{}://{}:{}", self.scheme, self.host, self.port),
        }
    }
}

/// A shared parse component, or a provider-local function passed directly
/// (same `(body, default_scheme) -> proxies` shape either way).
pub type ParseFn = for<'a> fn(&'a str, Option<Scheme>) -> Vec<ProxyRecord>;

#[derive(Clone, Copy)]
pub enum ParseKind {
    Entries,
    Json,
    Ndjson,
    Auto,
    Custom(ParseFn),
}

impl std::fmt::Debug for ParseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ParseKind::Entries => "entries",
            ParseKind::Json => "json",
            ParseKind::Ndjson => "ndjson",
            ParseKind::Auto => "auto",
            ParseKind::Custom(_) => "custom",
        })
    }
}

/// One declarative HTTP request belonging to a provider.
#[derive(Clone, Debug)]
pub struct Request {
    pub url: String,
    /// sub-feed name for stats/errors, e.g. "socks5", "page=30".
    pub label: String,
    /// default protocol for entries that carry none; None = the payload must
    /// self-describe its scheme (scheme-prefixed lines, json records).
    pub scheme: Option<Scheme>,
    pub parser: ParseKind,
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn new(url: impl Into<String>, label: impl Into<String>) -> Self {
        Request {
            url: url.into(),
            label: label.into(),
            scheme: None,
            parser: ParseKind::Auto,
            headers: Vec::new(),
        }
    }
    pub fn with(mut self, scheme: Option<Scheme>, parser: ParseKind) -> Self {
        self.scheme = scheme;
        self.parser = parser;
        self
    }
}

/// Per-provider result of one scrape pass.
#[derive(Debug)]
pub struct ScrapeOutcome {
    pub provider_id: &'static str,
    pub proxies: Vec<ProxyRecord>,
    pub requests_ok: usize,
    pub requests_total: usize,
    pub errors: Vec<String>,
}

/// An identified proxy source. Providers are stateless (`Sync` + no async
/// trait methods); the async pipeline lives in `runner::scrape`.
pub trait Provider: Send + Sync {
    /// unique lowercase id; the CLI selects and reports by it.
    fn id(&self) -> &'static str;
    /// human-facing page for --list-providers (owned: composed providers
    /// compute it from a repo field).
    fn site(&self) -> String {
        String::new()
    }
    /// comma-joined protocol list for --list-providers.
    fn protocols(&self) -> String {
        String::new()
    }
    /// cadence note from the source catalog.
    fn refresh(&self) -> &'static str {
        ""
    }
    /// safety cap per feed chain in advance().
    fn max_requests(&self) -> usize {
        64
    }
    /// per-provider wall clock; deep feeds under rate-limit pressure
    /// truncate loudly with partial data kept instead of stalling the run.
    fn time_budget(&self) -> Duration {
        Duration::from_secs(150)
    }

    /// Declarative seed plan; advance() pages each seed to exhaustion.
    fn requests(&self) -> Vec<Request>;

    /// Pagination hook: given the request just fetched and its body, return
    /// the next request for that feed, or None when exhausted.
    fn advance(&self, _req: &Request, _body: &str) -> Option<Request> {
        None
    }
}
// Wiring note: `providers::all()` returns `Vec<Arc<dyn Provider>>` directly;
// a factory-type alias proved unnecessary once the table module arrived.
