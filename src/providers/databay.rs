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
    fn advance(&self, req: &Request, body: &str) -> Result<Option<Request>, String> {
        let url = url::Url::parse(&req.url)
            .map_err(|_| "databay pagination parse failed: invalid request URL".to_string())?;
        let (mut proto, mut page): (Option<String>, Option<usize>) = (None, None);
        for (k, v) in url.query_pairs() {
            match &*k {
                "protocol" => proto = Some(v.into_owned()),
                "page" => page = v.parse().ok(),
                _ => {}
            }
        }
        let proto =
            proto.ok_or_else(|| "databay pagination parse failed: missing protocol".to_string())?;
        let page =
            page.ok_or_else(|| "databay pagination parse failed: invalid page".to_string())?;
        let value: serde_json::Value = serde_json::from_str(body)
            .map_err(|error| format!("databay pagination parse failed: {error}"))?;
        let total = value
            .get("total")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "databay pagination parse failed: missing total".to_string())?
            as usize;
        let next = page + 1;
        if page * LIMIT >= total {
            return Ok(None);
        }
        Ok(Some(
            Request::new(page_url(&proto, next), format!("{proto} p{next}")).with(
                PROTOCOLS
                    .iter()
                    .find(|(_, p)| *p == proto.as_str())
                    .map(|(s, _)| *s),
                ParseKind::Json,
            ),
        ))
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

        let second = provider
            .advance(&first, r#"{"total":1001}"#)
            .unwrap()
            .unwrap();
        assert_eq!(
            second.url,
            "https://databay.com/api/v1/proxy-list?protocol=socks4&limit=1000&page=2&format=json"
        );
        assert_eq!(second.label, "socks4 p2");
        assert_eq!(second.scheme, Some(Scheme::Socks4));
        assert_eq!(format!("{:?}", second.parser), "json");
        assert!(matches!(
            provider.advance(&second, r#"{"total":2000}"#),
            Ok(None)
        ));
    }

    #[test]
    fn advance_rejects_bad_bodies_without_claiming_exhaustion() {
        let provider = Databay;
        let first = provider.requests().remove(2);

        for body in [r#"{"total":"#, r#"{"error":"unavailable"}"#, "{}"] {
            let error = provider.advance(&first, body).unwrap_err();
            assert!(
                error.starts_with("databay pagination parse failed:"),
                "unexpected error: {error}"
            );
        }
    }
}
