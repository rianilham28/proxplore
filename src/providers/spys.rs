//! SPYS — the operator's TXT mirror, not the HTML pool.
//!
//! spys.one obfuscates ports behind inline JS for non-browser clients; the
//! operator publishes a JS-free mirror at spys.me/proxy.txt (live, near
//! real-time). Lines are bare ip:port (auth-bearing ones are ip:port:user:pass
//! — the entries parser handles both), so the scheme default is http.
//! Undeclared entries default to http.

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
        ]
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(Spys)
}
