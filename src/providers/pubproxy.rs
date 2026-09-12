//! PubProxy — tiny keyless API, frequent scans.
//!
//! Small pool, so one request is the whole feed. ``format=txt`` is asked for
//! yet the endpoint has historically answered TXT or JSON regardless, so the
//! shape-sniffing Auto component absorbs the flavor change; undeclared
//! entries default to http. The site itself is HTTP-only.

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Request, Scheme};

pub struct PubProxy;

impl Provider for PubProxy {
    fn id(&self) -> &'static str {
        "pubproxy"
    }
    fn site(&self) -> String {
        "http://pubproxy.com".into()
    }
    fn protocols(&self) -> String {
        "http,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "frequent"
    }
    fn requests(&self) -> Vec<Request> {
        vec![
            Request::new(
                "http://pubproxy.com/api/proxy?limit=100&format=txt",
                "limit=100",
            )
            .with(Some(Scheme::Http), ParseKind::Auto),
        ]
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(PubProxy)
}
