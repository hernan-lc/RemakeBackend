//! Opaque keyset cursors for the v3 list routes owned by one account.
//!
//! A cursor names the last row of a page. It is bound to the list it came from
//! (`scope`) and to the account that received it (`owner`), so a cursor copied
//! from another list or another account is refused as `invalidRequest` instead
//! of silently producing a different page. Cursors are not a security boundary:
//! every query is already scoped to the authenticated owner.

use crate::{
    api_error::{Kind, RequestId, V3Error},
    catalog_v3::{parse_limit, query_map},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Wire {
    v: u8,
    k: String,
    o: String,
    t: Option<String>,
    id: Option<String>,
    n: Option<i64>,
}

/// Position of the last row served: newest-first by time then id, or a number.
#[derive(Debug, PartialEq, Eq)]
pub enum Position {
    Time(DateTime<Utc>, Uuid),
    Number(i64),
}

fn owner_tag(owner: &str) -> String {
    hex::encode(&Sha256::digest(format!("cursor-owner:{owner}").as_bytes())[..8])
}

pub fn encode(scope: &str, owner: &str, position: &Position) -> String {
    let (t, id, n) = match position {
        Position::Time(at, id) => (
            Some(at.to_rfc3339_opts(SecondsFormat::Micros, true)),
            Some(id.to_string()),
            None,
        ),
        Position::Number(n) => (None, None, Some(*n)),
    };
    let wire = Wire {
        v: 1,
        k: scope.to_owned(),
        o: owner_tag(owner),
        t,
        id,
        n,
    };
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&wire).unwrap_or_default())
}

pub fn decode(
    rid: &RequestId,
    raw: &str,
    scope: &str,
    owner: &str,
    numeric: bool,
) -> Result<Position, V3Error> {
    let bad = || V3Error::new(rid, Kind::InvalidRequest);
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| bad())?;
    let wire: Wire = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    if wire.v != 1 || wire.k != scope || wire.o != owner_tag(owner) {
        return Err(bad());
    }
    match (numeric, wire.t, wire.id, wire.n) {
        (true, None, None, Some(n)) if n >= 1 => Ok(Position::Number(n)),
        (false, Some(t), Some(id), None) => {
            let at = DateTime::parse_from_rfc3339(&t)
                .map_err(|_| bad())?
                .with_timezone(&Utc);
            Ok(Position::Time(at, Uuid::parse_str(&id).map_err(|_| bad())?))
        }
        _ => Err(bad()),
    }
}

/// `limit` (1..=50, default 50) and the raw `cursor` of a list request. Any
/// other or repeated query key is refused.
pub fn request(
    rid: &RequestId,
    raw_query: Option<String>,
) -> Result<(i64, Option<String>), V3Error> {
    let params = query_map(rid, raw_query, &["limit", "cursor"])?;
    let limit = parse_limit(rid, params.get("limit"))?;
    Ok((limit, params.get("cursor").cloned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid() -> RequestId {
        RequestId("r".into())
    }

    #[test]
    fn time_and_number_cursors_round_trip_at_microsecond_precision() {
        let at = DateTime::parse_from_rfc3339("2026-11-11T10:20:30.123456Z")
            .unwrap()
            .with_timezone(&Utc);
        let id = Uuid::new_v4();
        let raw = encode("assets", "u1", &Position::Time(at, id));
        assert_eq!(
            decode(&rid(), &raw, "assets", "u1", false).unwrap(),
            Position::Time(at, id)
        );
        let raw = encode("overlay-revisions:x", "u1", &Position::Number(7));
        assert_eq!(
            decode(&rid(), &raw, "overlay-revisions:x", "u1", true).unwrap(),
            Position::Number(7)
        );
    }

    #[test]
    fn foreign_scope_owner_shape_and_garbage_are_refused() {
        let at = Utc::now();
        let raw = encode("assets", "u1", &Position::Time(at, Uuid::new_v4()));
        assert!(decode(&rid(), &raw, "overlays", "u1", false).is_err());
        assert!(decode(&rid(), &raw, "assets", "u2", false).is_err());
        assert!(decode(&rid(), &raw, "assets", "u1", true).is_err());
        assert!(decode(&rid(), "not base64 !!", "assets", "u1", false).is_err());
        let wrong_version = URL_SAFE_NO_PAD
            .encode(br#"{"v":2,"k":"assets","o":"0000000000000000","t":null,"id":null,"n":null}"#);
        assert!(decode(&rid(), &wrong_version, "assets", "u1", false).is_err());
        let zero = encode("s", "u1", &Position::Number(0));
        assert!(decode(&rid(), &zero, "s", "u1", true).is_err());
    }

    #[test]
    fn requests_accept_only_limit_and_cursor() {
        assert_eq!(request(&rid(), None).unwrap(), (50, None));
        assert_eq!(
            request(&rid(), Some("limit=5&cursor=abc".into())).unwrap(),
            (5, Some("abc".into()))
        );
        for bad in [
            "limit=0",
            "limit=51",
            "limit=x",
            "limit=",
            "q=a",
            "limit=1&limit=2",
        ] {
            assert!(request(&rid(), Some(bad.into())).is_err(), "{bad}");
        }
    }
}
