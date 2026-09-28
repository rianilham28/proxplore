//! SPYS — the operator's TXT mirror, not the HTML pool.
//!
//! spys.one obfuscates ports behind inline JS for non-browser clients; the
//! operator publishes a JS-free mirror at spys.me/proxy.txt (live, near
//! real-time). Lines are bare ip:port (auth-bearing ones are ip:port:user:pass
//! — the entries parser handles both), so the scheme default is http.
//! Undeclared entries default to http.
//!
//! A second mirror, spys.me/socks.txt, serves the operator's SOCKS list in the
//! same bare ip:port shape (probe-verified 2026-09-28: 400 rows). The two
//! mirrors cross-link in their headers (proxy.txt names socks.txt as
//! `Socks proxy=…`), but neither self-labels; the port mix is the real
//! evidence — 1080 ×215, 9050 ×34, 4145 ×23 at probe time, all canonical
//! SOCKS ports. It never prefixes a scheme, so the request defaults every
//! row to socks5 — the default, not an override, is what makes bare lines
//! parse as SOCKS.

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
        "http,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "near-realtime"
    }
    fn requests(&self) -> Vec<Request> {
        vec![
            Request::new("http://spys.me/proxy.txt", "txt")
                .with(Some(Scheme::Http), ParseKind::Entries),
            Request::new("http://spys.me/socks.txt", "socks5")
                .with(Some(Scheme::Socks5), ParseKind::Entries),
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
    fn requests_cover_both_probe_verified_mirrors() {
        // Row count grows only when a probe-verified mirror survives: 1 probe
        // (2026-09-28) found socks.txt 200/400 tokens and three 404s.
        let requests = Spys.requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].url, "http://spys.me/proxy.txt");
        assert_eq!(requests[0].label, "txt");
        assert_eq!(requests[0].scheme, Some(Scheme::Http));
        assert!(matches!(requests[0].parser, ParseKind::Entries));
        assert_eq!(requests[1].url, "http://spys.me/socks.txt");
        assert_eq!(requests[1].label, "socks5");
        assert_eq!(
            requests[1].scheme,
            Some(Scheme::Socks5),
            "bare ip:port lines never carry a scheme, so the request default \
             is the only thing that labels this SOCKS pool"
        );
        assert!(matches!(requests[1].parser, ParseKind::Entries));
    }
}
