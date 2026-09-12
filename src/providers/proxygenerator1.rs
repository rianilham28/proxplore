//! proxygenerator1/ProxyGenerator — ALL/all.json, the evidence-quality feed:
//! an address enters only after carrying real traffic to named sites (16-site
//! access matrix per record), every record dated.
//!
//! ~2.4k protocol-labelled records ride the shared "json" parser (protocol
//! and scheme keys). ALL/ALL.txt is the same pool untagged and stays unwired.
//! meta.json flags auth_required records that carry no user:pass fields —
//! upstream-withheld credentials, so nothing is silently dropped and this is
//! not a credential-leak feed.

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Request};

const URL: &str =
    "https://raw.githubusercontent.com/proxygenerator1/ProxyGenerator/main/ALL/all.json";

pub struct ProxyGenerator1;

impl Provider for ProxyGenerator1 {
    fn id(&self) -> &'static str {
        "proxygenerator1"
    }
    fn site(&self) -> String {
        "https://github.com/proxygenerator1/ProxyGenerator".into()
    }
    fn protocols(&self) -> String {
        "http,https,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "per rebuild (daily)"
    }
    fn requests(&self) -> Vec<Request> {
        vec![Request::new(URL, "all.json").with(None, ParseKind::Json)]
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(ProxyGenerator1)
}
