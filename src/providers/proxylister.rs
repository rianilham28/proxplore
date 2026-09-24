//! Proxylister — keyless JSON API (~17k checked relays with geo/uptime
//! metadata; the only new §1-class API found by the expansion sweep).
//!
//! ``limit`` hard-caps at 500 (1000 -> 422) and ``next`` is null on every
//! page, so pagination is count-driven: advance() walks pages until
//! page*500 >= the envelope's ``count`` (max_requests bounds the walk at
//! 40). Records declare their own ``protocols`` lists — a meaningful share is
//! socks-only — so Requests carry NO default scheme.

use std::sync::Arc;
use std::time::Duration;

use crate::model::{ParseKind, Provider, Request};

const API: &str = "https://proxylister.com/api/v1/proxies";
const PAGE_SIZE: usize = 500;

pub struct Proxylister;

fn page_url(page: usize) -> String {
    format!("{API}?limit={PAGE_SIZE}&page={page}&sort=-last_checked_at")
}

impl Provider for Proxylister {
    fn id(&self) -> &'static str {
        "proxylister"
    }
    fn site(&self) -> String {
        "https://proxylister.com/".into()
    }
    fn protocols(&self) -> String {
        "http,socks4,socks5".into()
    }
    fn refresh(&self) -> &'static str {
        "continuous checks"
    }
    fn max_requests(&self) -> usize {
        40 // 500/page, count ~17k, bounded politely
    }
    fn time_budget(&self) -> Duration {
        Duration::from_secs(240)
    }
    fn requests(&self) -> Vec<Request> {
        vec![Request::new(page_url(1), "p1").with(None, ParseKind::Json)]
    }
    fn advance(&self, req: &Request, body: &str) -> Result<Option<Request>, String> {
        let url = url::Url::parse(&req.url)
            .map_err(|_| "proxylister pagination parse failed: invalid request URL".to_string())?;
        let page: usize = url
            .query_pairs()
            .find(|(k, _)| k == "page")
            .and_then(|(_, value)| value.parse().ok())
            .ok_or_else(|| "proxylister pagination parse failed: invalid page".to_string())?;
        let value: serde_json::Value = serde_json::from_str(body)
            .map_err(|error| format!("proxylister pagination parse failed: {error}"))?;
        let count = value
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "proxylister pagination parse failed: missing count".to_string())?
            as usize;
        let next = page + 1;
        if page * PAGE_SIZE >= count {
            return Ok(None);
        }
        Ok(Some(
            Request::new(page_url(next), format!("p{next}")).with(None, ParseKind::Json),
        ))
    }
}

pub fn new() -> Arc<dyn Provider> {
    Arc::new(Proxylister)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_continues_before_count_and_stops_on_last_page() {
        let provider = Proxylister;
        let first = provider.requests().remove(0);

        let second = provider
            .advance(&first, r#"{"count":501}"#)
            .unwrap()
            .unwrap();
        assert_eq!(
            second.url,
            "https://proxylister.com/api/v1/proxies?limit=500&page=2&sort=-last_checked_at"
        );
        assert_eq!(second.label, "p2");
        assert_eq!(second.scheme, None);
        assert_eq!(format!("{:?}", second.parser), "json");
        assert!(matches!(
            provider.advance(&second, r#"{"count":1000}"#),
            Ok(None)
        ));
    }

    #[test]
    fn advance_rejects_bad_bodies_without_claiming_exhaustion() {
        let provider = Proxylister;
        let first = provider.requests().remove(0);

        for body in [r#"{"count":"#, r#"{"error":"unavailable"}"#, "{}"] {
            let error = provider.advance(&first, body).unwrap_err();
            assert!(
                error.starts_with("proxylister pagination parse failed:"),
                "unexpected error: {error}"
            );
        }
    }
}
