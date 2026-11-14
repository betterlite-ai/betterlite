use super::error::ProjectionError;
use crate::scim_records::raw_object::RawObject;
use betterlite_db::budget_permission_rows::Timestamp;
use chrono::Timelike as _;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Meta<'kind> {
    resource_type: &'kind str,
    created: Option<String>,
    last_modified: Option<String>,
}

// Python's datetime.isoformat: the fraction appears only when microseconds are not zero, and a naive value has no offset
#[must_use]
pub fn isoformat(time: Timestamp) -> String {
    let (local, offset) = match time {
        Timestamp::Aware(aware) => (aware.naive_local(), aware.offset().to_string()),
        Timestamp::Naive(naive) => (naive, String::new()),
    };
    let pattern = if local.nanosecond() < 1_000 {
        "%Y-%m-%dT%H:%M:%S"
    } else {
        "%Y-%m-%dT%H:%M:%S%.6f"
    };
    format!("{}{offset}", local.format(pattern))
}

pub fn resource_meta(
    resource_type: &str,
    created: Option<Timestamp>,
    updated: Option<Timestamp>,
) -> Result<RawObject, ProjectionError> {
    let meta = Meta {
        resource_type,
        created: created.map(isoformat),
        last_modified: updated.map(isoformat),
    };
    let encode = |source: serde_json::Error| ProjectionError::Encode { source };
    serde_json::from_str(&serde_json::to_string(&meta).map_err(encode)?).map_err(encode)
}
