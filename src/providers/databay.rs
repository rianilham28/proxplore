//! Databay — JSON proxy-list API, five-minute refresh.
//!
//! ``limit=1000`` (API max) per page; the JSON envelope's ``total`` drives
//! advance() so paging stops exactly at the last page — full harvest per
//! protocol. Records carry ip/port/protocol plus optional username/password,
//! read field-agnostically by the shared "json" component. TXT hotlinks
//! exist but the JSON API is structured and credentialed, so it is the lane.

use std::sync::Arc;

use crate::model::{ParseKind, Provider, Request, Scheme};

const API: &str = "https://databay.com/api/v1/proxy-list";
const LIMIT: usize = 1000;

const PROTOCOLS: [(Scheme, &str); 4] = [
    (Scheme::Http, "http"),
    (Scheme::Https, "https"),
    (Scheme::Socks4, "socks4"),
    (Scheme::Socks5, "socks5"),
];

pub struct Databay;

fn page_url(proto: &str, page: usize) -> String {
    format!("{API}?protocol={proto}&limit={LIMIT}&page={page}&format=json")
}

impl Provider for Databay {
    fn id(&self) -> &'static str {
        "databay"
    }
    fn site(&self) -> String {
        "https://databay.com/free-proxy-list".into()
    }
    fn protocols(&self) -> String {
        "http,https,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "5 min"
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
                "protocol" => proto = Some(v.into_owned()),
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
        if page * LIMIT >= total {
            return None;
        }
        let _ = next - 1;
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
    Arc::new(Databay)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_continues_before_total_and_stops_on_last_page() {
        let provider = Databay;
        let first = provider.requests().remove(2);

        let second = provider.advance(&first, r#"{"total":1001}"#).unwrap();
        assert_eq!(
            second.url,
            "https://databay.com/api/v1/proxy-list?protocol=socks4&limit=1000&page=2&format=json"
        );
        assert_eq!(second.label, "socks4 p2");
        assert_eq!(second.scheme, Some(Scheme::Socks4));
        assert_eq!(format!("{:?}", second.parser), "json");
        assert!(provider.advance(&second, r#"{"total":2000}"#).is_none());
    }
}
