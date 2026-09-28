//! ProxyScrape — keyless v4 public API, near-realtime pool.
//!
//! Four requests over three protocols (``protocol=`` filters the feed; the
//! ``proxytype`` spelling is silently ignored); ``format=text`` +
//! ``proxy_format=ipport`` yields bare ip:port lines, so the default scheme is
//! authoritative and the shared entries component covers it. The https lane is
//! the http feed re-filtered with ``ssl=yes`` — a yes/no boolean, not
//! ``true`` (``ssl=true`` is a 400) — and that ssl-filtered slice is not a
//! subset of the unfiltered http pool. The v4 endpoint returns the whole
//! filtered pool in one response (no paging); the HTML page is a SPA.

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Request, Scheme};

const API: &str = "https://api.proxyscrape.com/v4/free-proxy-list/get";
const PROTOCOLS: [(Scheme, &str, &str); 4] = [
    (Scheme::Http, "http", "protocol=http"),
    (Scheme::Https, "https", "protocol=http&ssl=yes"),
    (Scheme::Socks4, "socks4", "protocol=socks4"),
    (Scheme::Socks5, "socks5", "protocol=socks5"),
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
        "http,https,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "~1 min"
    }
    fn requests(&self) -> Vec<Request> {
        PROTOCOLS
            .iter()
            .map(|(scheme, label, query)| {
                Request::new(
                    format!(
                        "{API}?request=display_proxies&{query}&proxy_format=ipport&format=text"
                    ),
                    *label,
                )
                .with(Some(*scheme), ParseKind::Entries)
            })
            .collect()
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(ProxyScrape)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_cover_http_https_slice_and_socks_lanes() {
        let provider = ProxyScrape;
        let requests = provider.requests();
        let labels: Vec<&str> = requests.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["http", "https", "socks4", "socks5"]);
        assert_eq!(provider.protocols(), "http,https,socks4,socks5");
        let https_req = &requests[1];
        let expected = format!(
            "{API}?request=display_proxies&protocol=http&ssl=yes&proxy_format=ipport&format=text"
        );
        assert_eq!(https_req.url, expected);
        assert_eq!(https_req.scheme, Some(Scheme::Https));
        assert_eq!(https_req.label, "https");
    }
}
