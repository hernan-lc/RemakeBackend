//! ICE discovery configuration on the existing authenticated HTTP API.
//! STUN only discovers an address; it does not relay application traffic.
use crate::api_error::Authenticated;
use axum::{extract::State, Json};
use serde_json::{json, Value};

pub(super) fn parse(raw: Option<String>) -> Result<Vec<String>, String> {
    let mut urls = Vec::new();
    for value in raw
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let host = value
            .strip_prefix("stun:")
            .ok_or("RTC_STUN_URLS requires stun: URLs")?;
        let url = url::Url::parse(&format!("udp://{host}"))
            .map_err(|_| "RTC_STUN_URLS contains an invalid URL")?;
        if value.len() > 2048
            || value.chars().any(char::is_control)
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || !url.path().is_empty()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
        {
            return Err(
                "RTC_STUN_URLS requires STUN hosts without credentials or query parameters".into(),
            );
        }
        if !urls.iter().any(|s| s == value) {
            urls.push(value.to_owned());
        }
        if urls.len() > 8 {
            return Err("RTC_STUN_URLS supports at most eight servers".into());
        }
    }
    Ok(urls)
}
pub async fn configuration(
    Authenticated(_): Authenticated,
    State(state): State<crate::AppState>,
) -> Json<Value> {
    Json(json!({"protocolVersion":1,"stunUrls":state.peer.stun_urls}))
}
#[cfg(test)]
mod tests {
    use super::parse;
    #[test]
    fn stun_configuration_is_explicit_bounded_and_has_no_credentials() {
        assert!(parse(None).unwrap().is_empty());
        assert_eq!(
            parse(Some(
                "stun:example.test:3478,stun:example.test:3478,stun:[::1]:3478".into()
            ))
            .unwrap()
            .len(),
            2
        );
        for raw in [
            "stun:",
            "stun:example.test:0",
            "turn:example.test:3478",
            "stun:user:secret@example.test:3478",
            "stun:example.test?token=x",
            "stun:example.test/path",
            "https://example.test",
        ] {
            assert!(parse(Some(raw.into())).is_err());
        }
        assert!(parse(Some(
            (1..=9)
                .map(|n| format!("stun:s{n}.example.test:3478"))
                .collect::<Vec<_>>()
                .join(",")
        ))
        .is_err());
    }
}
