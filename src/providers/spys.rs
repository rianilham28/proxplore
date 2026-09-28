//! SPYS — the operator's TXT mirror, not the HTML pool.
//!
//! spys.one obfuscates ports behind inline JS for non-browser clients; the
//! operator publishes a JS-free mirror at spys.me/proxy.txt (live, near
//! real-time). Lines are bare ip:port (auth-bearing ones are ip:port:user:pass
//! — the entries parser handles both), so the scheme default is http.
//! Undeclared entries default to http.
//!
//! A second mirror, spys.me/socks.txt, serves the sibling SOCKS list in the
//! same bare ip:port shape (probe-verified 2026-09-28). It declares no
//! scheme prefixes, so it is fetched with the same http default.

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Request, Scheme};

pub struct Spys;

impl Provider for Spys {
    fn id(&self) -> &'static str {
        "spys"
    }
    fn site(&self) -> String {
        "https://www.spys.one/en/".into()
    }
    fn protocols(&self) -> String {
        "http".into()
    }
    fn refresh(&self) -> &'static str {
        "near-realtime"
    }
    fn requests(&self) -> Vec<Request> {
        vec![
            Request::new("http://spys.me/proxy.txt", "txt")
                .with(Some(Scheme::Http), ParseKind::Entries),
            Request::new("http://spys.me/socks.txt", "socks")
                .with(Some(Scheme::Http), ParseKind::Entries),
        ]
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(Spys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_txt_mirror_with_http_default() {
        let requests = Spys.requests();
        assert_eq!(requests.len(), 2); // 1 probe-verified mirror survives
        assert_eq!(requests[0].url, "http://spys.me/proxy.txt");
        assert_eq!(requests[0].label, "txt");
        assert!(matches!(requests[0].parser, ParseKind::Entries));
        assert_eq!(requests[1].url, "http://spys.me/socks.txt");
        assert_eq!(requests[1].label, "socks");
        assert!(matches!(requests[1].parser, ParseKind::Entries));
    }
}
