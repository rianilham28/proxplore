//! GeoNode — JSON proxy-list API.
//!
//! The endpoint 403s non-browser clients; wreq Chrome emulation satisfies it
//! with no extra headers. One seed per protocol (the ``protocols=`` filter
//! decides the scheme) and advance() pages each seed off the envelope's
//! ``total`` — the API caps ``limit`` at 500, so full harvest means paging.
//! (Reference provider for the total-driven-paging pattern.)

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Request, Scheme};

const API: &str = "https://proxylist.geonode.com/api/proxy-list";
const PAGE_SIZE: usize = 500; // API hard cap: "Max limit on 'limit' value is 500"

const PROTOCOLS: [(Scheme, &str); 4] = [
    (Scheme::Http, "http"),
    (Scheme::Https, "https"),
    (Scheme::Socks4, "socks4"),
    (Scheme::Socks5, "socks5"),
];

pub struct GeoNode;

fn page_url(proto: &str, page: usize) -> String {
    format!(
        "{API}?limit={PAGE_SIZE}&page={page}&sort_by=lastChecked&sort_type=desc&protocols={proto}"
    )
}

impl Provider for GeoNode {
    fn id(&self) -> &'static str {
        "geonode"
    }
    fn site(&self) -> String {
        "https://www.geonode.com/".into()
    }
    fn protocols(&self) -> String {
        "http,https,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "continuous"
    }
    fn requests(&self) -> Vec<Request> {
        PROTOCOLS
            .iter()
            .map(|(scheme, proto)| {
                Request::new(page_url(proto, 1), format!("{proto} p1"))
                    .with(Some(*scheme), ParseKind::Json)
            })
            .collect()
    }
    fn advance(&self, req: &Request, body: &str) -> Option<Request> {
        let url = url::Url::parse(&req.url).ok()?;
        let (mut proto, mut page): (Option<String>, Option<usize>) = (None, None);
        for (k, v) in url.query_pairs() {
            match &*k {
                "protocols" => proto = Some(v.into_owned()),
                "page" => page = v.parse().ok(),
                _ => {}
            }
        }
        let (proto, page) = (proto?, page?);
        let total = serde_json::from_str::<serde_json::Value>(body)
            .ok()?
            .get("total")?
            .as_u64()? as usize;
        let next = page + 1;
        if (next - 1) * PAGE_SIZE >= total {
            return None;
        }
        Some(
            Request::new(page_url(&proto, next), format!("{proto} p{next}")).with(
                PROTOCOLS
                    .iter()
                    .find(|(_, p)| *p == proto.as_str())
                    .map(|(s, _)| *s),
                ParseKind::Json,
            ),
        )
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(GeoNode)
}
