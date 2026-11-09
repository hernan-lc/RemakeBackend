//! Operator-only audit and compare-and-swap repair of derived JSONB digests.
//! No HTTP route invokes these helpers; migration never rewrites old rows.
use crate::{library, portable};
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditCursor {
    pub document_id: Uuid,
    pub revision: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditItem {
    pub document_id: Uuid,
    pub revision: i64,
    pub status: String,
    pub stored_sha256: String,
    pub normalized_sha256: String,
    pub portable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPage {
    pub items: Vec<AuditItem>,
    pub next_after: Option<AuditCursor>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairReceipt {
    pub document_id: Uuid,
    pub revision: i64,
    pub old_sha256: String,
    pub normalized_sha256: String,
}

#[derive(Debug)]
pub enum RepairError {
    InvalidInput,
    NotFound,
    Conflict,
    InvalidPortable,
    Database(sqlx::Error),
}
impl std::fmt::Display for RepairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "invalid library digest audit/repair arguments",
            Self::NotFound => "library document revision not found",
            Self::Conflict => "library digest changed; audit and review this revision again",
            Self::InvalidPortable => "stored document is not portable; digest repair refused",
            Self::Database(_) => "library digest audit/repair database operation failed",
        })
    }
}
impl std::error::Error for RepairError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}
impl From<sqlx::Error> for RepairError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// Scans at most `limit` revisions and emits only invalid identities. The
/// cursor is the last scanned row, even when the page has no mismatches.
pub async fn audit(
    pool: &PgPool,
    after: Option<AuditCursor>,
    limit: u32,
) -> Result<AuditPage, RepairError> {
    if !(1..=100).contains(&limit) || after.is_some_and(|cursor| cursor.revision <= 0) {
        return Err(RepairError::InvalidInput);
    }
    let rows = sqlx::query("SELECT d.id,d.kind,v.revision,v.status,v.document,v.sha256 FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE ($1::uuid IS NULL OR (d.id,v.revision)>($1,$2::bigint)) ORDER BY d.id,v.revision LIMIT $3")
        .bind(after.map(|cursor| cursor.document_id))
        .bind(after.map_or(0, |cursor| cursor.revision))
        .bind(i64::from(limit) + 1).fetch_all(pool).await?;
    let more = rows.len() > limit as usize;
    let mut items = Vec::new();
    let mut last = None;
    for row in rows.iter().take(limit as usize) {
        let document_id = row.get("id");
        let revision = row.get("revision");
        last = Some(AuditCursor {
            document_id,
            revision,
        });
        let document: Value = row.get("document");
        let stored_sha256: String = row.get("sha256");
        let normalized_sha256 = library::digest(&document);
        let portable = portable::document(&row.get::<String, _>("kind"), &document);
        if stored_sha256 != normalized_sha256 || !portable {
            items.push(AuditItem {
                document_id,
                revision,
                status: row.get("status"),
                stored_sha256,
                normalized_sha256,
                portable,
            });
        }
    }
    Ok(AuditPage {
        items,
        next_after: if more { last } else { None },
    })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Repairs only a reviewed derived digest. Both expected hashes bind the
/// operator's audit to the locked row; a changed source cannot be accepted.
pub async fn repair(
    pool: &PgPool,
    document_id: Uuid,
    revision: i64,
    expected_old_sha256: &str,
    expected_normalized_sha256: &str,
    operator: &str,
) -> Result<RepairReceipt, RepairError> {
    if revision <= 0
        || !valid_sha256(expected_old_sha256)
        || !valid_sha256(expected_normalized_sha256)
        || expected_old_sha256 == expected_normalized_sha256
        || operator.is_empty()
        || operator.len() > 200
        || operator.trim() != operator
        || operator.chars().any(char::is_control)
    {
        return Err(RepairError::InvalidInput);
    }
    let mut tx = pool.begin().await?;
    let row = sqlx::query("SELECT d.kind,v.document,v.sha256 FROM cloud_library_documents d JOIN cloud_library_versions v ON v.document_id=d.id WHERE d.id=$1 AND v.revision=$2 FOR UPDATE OF d,v")
        .bind(document_id).bind(revision).fetch_optional(&mut *tx).await?.ok_or(RepairError::NotFound)?;
    let document: Value = row.get("document");
    if row.get::<String, _>("sha256") != expected_old_sha256
        || library::digest(&document) != expected_normalized_sha256
    {
        return Err(RepairError::Conflict);
    }
    if !portable::document(&row.get::<String, _>("kind"), &document) {
        return Err(RepairError::InvalidPortable);
    }
    sqlx::query("UPDATE cloud_library_versions SET sha256=$3 WHERE document_id=$1 AND revision=$2")
        .bind(document_id)
        .bind(revision)
        .bind(expected_normalized_sha256)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO cloud_library_digest_repairs(id,document_id,revision,old_sha256,normalized_sha256,operator) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(Uuid::new_v4()).bind(document_id).bind(revision).bind(expected_old_sha256)
        .bind(expected_normalized_sha256).bind(operator).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(RepairReceipt {
        document_id,
        revision,
        old_sha256: expected_old_sha256.into(),
        normalized_sha256: expected_normalized_sha256.into(),
    })
}
