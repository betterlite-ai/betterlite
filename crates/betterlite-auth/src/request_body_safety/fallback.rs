use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::request_ban_catalog::RequestBanCatalog;

use super::{RequestBodySafety, SafetyError, banned, destination};

const FALLBACK_FIELDS: [&str; 3] = [
    "fallbacks",
    "context_window_fallbacks",
    "content_policy_fallbacks",
];

// LiteLLM's iter_request_fallback_targets over the root and a mapping router_settings_override, checked in
// the order the generator yields: a mapping target before the fallbacks nested inside it.
pub fn check(scan: &RequestBodySafety<'_>, catalog: &RequestBanCatalog) -> Result<(), SafetyError> {
    let override_fields = match scan.body.get("router_settings_override") {
        Some(Json::Object(fields)) => Some(fields),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    };
    for source in [Some(scan.body), override_fields] {
        let Some(fields) = source else {
            continue;
        };
        for field in FALLBACK_FIELDS {
            targets(scan, catalog, fields.get(field), 0)?;
        }
    }
    Ok(())
}

// The depth check runs before the type check, so an absent or scalar value at an excessive depth fails too.
fn targets(
    scan: &RequestBodySafety<'_>,
    catalog: &RequestBanCatalog,
    value: Option<&Json>,
    depth: i64,
) -> Result<(), SafetyError> {
    if depth > scan.max_fallbacks.saturating_mul(2) {
        return Err(SafetyError::FallbackDepth);
    }
    let Some(Json::List(items)) = value else {
        return Ok(());
    };
    for item in items {
        match item {
            Json::Text(text) => destination::reject(text)?,
            Json::Object(fields) => mapping(scan, catalog, fields, depth)?,
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {}
        }
    }
    Ok(())
}

fn mapping(
    scan: &RequestBodySafety<'_>,
    catalog: &RequestBanCatalog,
    fields: &JsonObject,
    depth: i64,
) -> Result<(), SafetyError> {
    let values: Vec<&Json> = fields.values().collect();
    let all_lists = !values.is_empty() && values.iter().all(|value| matches!(value, Json::List(_)));
    if !all_lists {
        // a yielded mapping is scanned with the original request's model opt-ins, then its model must pass
        banned::check(scan, fields, catalog)?;
        if let Some(Json::Text(target)) = fields.get("model") {
            destination::reject(target)?;
        }
    }
    match fields.get("model") {
        Some(Json::Text(_)) => {
            // a mapping with a model string is traversed only through its three fallback fields
            for field in FALLBACK_FIELDS {
                targets(scan, catalog, fields.get(field), depth.saturating_add(1))?;
            }
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => {
            for value in fields.values() {
                targets(scan, catalog, Some(value), depth.saturating_add(1))?;
            }
        }
    }
    Ok(())
}
