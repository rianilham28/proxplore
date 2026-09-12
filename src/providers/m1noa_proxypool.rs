//! M1noa/proxypool — Go scraper publishing checked records on an `output`
//! branch (hourly force-push).
//!
//! proxies.json is a top-level JSON ARRAY of records; the shared "json"
//! parser unwraps arrays directly. Each record declares protocols[] (which
//! fans multi-protocol relays out); the separate `https:true` boolean is NOT
//! a protocol declaration and is deliberately not hand-fanned.

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Request};

const URL: &str = "https://raw.githubusercontent.com/M1noa/proxypool/output/proxies.json";

pub struct M1noaProxypool;

impl Provider for M1noaProxypool {
    fn id(&self) -> &'static str {
        "m1noa-proxypool"
    }
    fn site(&self) -> String {
        "https://github.com/M1noa/proxypool".into()
    }
    fn protocols(&self) -> String {
        "http,https,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "hourly (output branch)"
    }
    fn requests(&self) -> Vec<Request> {
        vec![Request::new(URL, "json").with(None, ParseKind::Json)]
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(M1noaProxypool)
}
