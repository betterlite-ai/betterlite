use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use secrecy::ExposeSecret;

use super::{BATCH_ENQUEUED_LIMIT_KEY, BatchEnqueuedPrincipal, BatchEnqueuedTokenScope, ScopeKey};

#[must_use]
pub fn resolve_enqueued_scopes(principal: &BatchEnqueuedPrincipal) -> Vec<BatchEnqueuedTokenScope> {
    let key_limit = read_metadata_limit(principal.metadata.as_ref());
    let team_limit = read_metadata_limit(principal.team_metadata.as_ref());
    let mut scopes = Vec::with_capacity(2);
    let key = principal
        .api_key
        .as_ref()
        .map(ExposeSecret::expose_secret)
        .filter(|value| !value.is_empty());
    if let (Some(limit), Some(value)) = (key_limit, key) {
        scopes.push(BatchEnqueuedTokenScope {
            key: ScopeKey::ApiKey,
            value: value.to_owned(),
            limit,
        });
    }
    let team = principal
        .team_id
        .as_deref()
        .filter(|value| !value.is_empty());
    if let (Some(limit), Some(value)) = (team_limit, team) {
        scopes.push(BatchEnqueuedTokenScope {
            key: ScopeKey::Team,
            value: value.to_owned(),
            limit,
        });
    }
    scopes
}

// the source reads a positive int through a pydantic adapter and disables the scope, with a warning, on anything else
fn read_metadata_limit(metadata: Option<&BTreeMap<String, JsonData>>) -> Option<i64> {
    let values = metadata.filter(|values| !values.is_empty())?;
    let raw = values.get(BATCH_ENQUEUED_LIMIT_KEY)?;
    match raw {
        JsonData::Null => None,
        JsonData::Number(number) => match number.as_i64() {
            Some(limit) if limit > 0 => Some(limit),
            _ => reject_limit(raw),
        },
        JsonData::Bool(_) | JsonData::String(_) | JsonData::Array(_) | JsonData::Object(_) => {
            reject_limit(raw)
        }
    }
}

fn reject_limit(raw: &JsonData) -> Option<i64> {
    tracing::warn!(
        value = ?raw,
        "ignoring an invalid batch_enqueued_token_limit metadata value; expected a positive integer"
    );
    None
}
