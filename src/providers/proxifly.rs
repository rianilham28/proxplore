//! Proxifly — jsDelivr-hosted feeds from proxifly/free-proxy-list.
//!
//! Lines are scheme-prefixed (``http://1.2.3.4:8080``), so the shared entries
//! parser reads the protocol from the line itself and the Request carries no
//! default scheme. ``@main`` is pinned; swapping it for a commit hash busts
//! the CDN cache. The hosted REST API needs a key and measured ~0% live rate,
//! so the CDN feed is the lane.

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Request};

const CDN: &str = "https://cdn.jsdelivr.net/gh/proxifly/free-proxy-list@main/proxies/protocols/";

pub struct Proxifly;

impl Provider for Proxifly {
    fn id(&self) -> &'static str {
        "proxifly"
    }
    fn site(&self) -> String {
        "https://proxifly.com/free-proxy-list".into()
    }
    fn protocols(&self) -> String {
        "http,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "5 min"
    }
    fn requests(&self) -> Vec<Request> {
        // scheme-prefixed lines: Request.scheme stays None (line wins)
        ["http", "socks4", "socks5"]
            .iter()
            .map(|proto| {
                Request::new(format!("{CDN}{proto}/data.txt"), *proto)
                    .with(None, ParseKind::Entries)
            })
            .collect()
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(Proxifly)
}
