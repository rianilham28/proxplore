//! ProxyScrape — keyless v4 public API, near-realtime pool.
//!
//! One request per protocol (``protocol=`` filters the feed; the ``proxytype``
//! spelling is silently ignored); ``format=text`` + ``proxy_format=ipport``
//! yields bare ip:port lines, so the default scheme is authoritative and the
//! shared entries component covers it. The v4 endpoint returns the whole
//! filtered pool in one response (no paging); the HTML page is a SPA.

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Request, Scheme};

const API: &str = "https://api.proxyscrape.com/v4/free-proxy-list/get";
const PROTOCOLS: [(Scheme, &str); 3] = [
    (Scheme::Http, "http"),
    (Scheme::Socks4, "socks4"),
    (Scheme::Socks5, "socks5"),
];

pub struct ProxyScrape;

impl Provider for ProxyScrape {
    fn id(&self) -> &'static str {
        "proxyscrape"
    }
    fn site(&self) -> String {
        "https://www.proxyscrape.com/free-proxy-list".into()
    }
    fn protocols(&self) -> String {
        "http,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "~1 min"
    }
    fn requests(&self) -> Vec<Request> {
        PROTOCOLS
            .iter()
            .map(|(scheme, proto)| {
                Request::new(
                    format!(
                        "{API}?request=display_proxies&protocol={proto}&proxy_format=ipport&format=text"
                    ),
                    *proto,
                )
                .with(Some(*scheme), ParseKind::Entries)
            })
            .collect()
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(ProxyScrape)
}
