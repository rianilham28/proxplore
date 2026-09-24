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
    fn advance(&self, req: &Request, body: &str) -> Result<Option<Request>, String> {
        let url = url::Url::parse(&req.url)
            .map_err(|_| "geonode pagination parse failed: invalid request URL".to_string())?;
        let (mut proto, mut page): (Option<String>, Option<usize>) = (None, None);
        for (k, v) in url.query_pairs() {
            match &*k {
                "protocols" => proto = Some(v.into_owned()),
                "page" => page = v.parse().ok(),
                _ => {}
            }
        }
        let proto =
            proto.ok_or_else(|| "geonode pagination parse failed: missing protocol".to_string())?;
        let page =
            page.ok_or_else(|| "geonode pagination parse failed: invalid page".to_string())?;
        let value: serde_json::Value = serde_json::from_str(body)
            .map_err(|error| format!("geonode pagination parse failed: {error}"))?;
        let total = value
            .get("total")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "geonode pagination parse failed: missing total".to_string())?
            as usize;
        let next = page + 1;
        if (next - 1) * PAGE_SIZE >= total {
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
    Arc::new(GeoNode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_continues_before_total_and_stops_on_last_page() {
        let provider = GeoNode;
        let first = provider.requests().remove(2);

        let second = provider
            .advance(&first, r#"{"total":501}"#)
            .unwrap()
            .unwrap();
        assert_eq!(
            second.url,
            "https://proxylist.geonode.com/api/proxy-list?limit=500&page=2&sort_by=lastChecked&sort_type=desc&protocols=socks4"
        );
        assert_eq!(second.label, "socks4 p2");
        assert_eq!(second.scheme, Some(Scheme::Socks4));
        assert_eq!(format!("{:?}", second.parser), "json");
        assert!(matches!(
            provider.advance(&second, r#"{"total":1000}"#),
            Ok(None)
        ));
    }

    #[test]
    fn advance_rejects_bad_bodies_without_claiming_exhaustion() {
        let provider = GeoNode;
        let first = provider.requests().remove(2);

        for body in [r#"{"total":"#, r#"{"error":"unavailable"}"#, "{}"] {
            let error = provider.advance(&first, body).unwrap_err();
            assert!(
                error.starts_with("geonode pagination parse failed:"),
                "unexpected error: {error}"
            );
        }
    }
}
