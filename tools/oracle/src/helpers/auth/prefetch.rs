use betterlite_auth::auth_prefetch::entries::{AuthObjectRefs, iter_entries};
use betterlite_auth::auth_prefetch::identity::{identity_cache_keys, identity_memory_ttl};
use betterlite_auth::auth_prefetch::rows::{RowKind, RowObject, validate_row};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = PrefetchInput;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(match input {
            PrefetchInput::ReferencesAndEntries {
                management_ttl,
                organization_ttl,
                tokens,
            } => references_and_entries(tokens, *management_ttl, *organization_ttl),
            PrefetchInput::IdentityCacheKeys { calls } => identity_keys(calls),
            PrefetchInput::RowValidation { refreshed_at, rows } => {
                row_validation(rows, *refreshed_at)
            }
            PrefetchInput::IdentityTtl { ttls } => identity_ttls(ttls),
        })
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PrefetchInput {
    ReferencesAndEntries {
        management_ttl: f64,
        organization_ttl: f64,
        tokens: Vec<LiteLLMVerificationToken>,
    },
    IdentityCacheKeys {
        calls: Vec<CacheKeyCall>,
    },
    RowValidation {
        refreshed_at: f64,
        rows: Vec<RowCall>,
    },
    IdentityTtl {
        ttls: Vec<TtlCall>,
    },
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheKeyCall {
    api_key: String,
    end_user_id: Option<String>,
    key_is_resolved: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RowCall {
    row: RowKind,
    value: Option<JsonData>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TtlCall {
    value: Json,
    management_ttl: f64,
}

fn references_and_entries(
    tokens: &[LiteLLMVerificationToken],
    management_ttl: f64,
    organization_ttl: f64,
) -> Value {
    Value::Array(
        tokens
            .iter()
            .map(|token| {
                let refs = AuthObjectRefs::from_token(token);
                let entries = iter_entries(&refs, management_ttl, organization_ttl);
                let entries: Vec<Value> = entries
                    .iter()
                    .map(|entry| {
                        json!({
                            "key": entry.key,
                            "row": entry.row.label(),
                            "model": entry.row.model_name(),
                            "ttl": entry.ttl,
                        })
                    })
                    .collect();
                json!({
                    "refs": {
                        "user_id": refs.user_id,
                        "team_id": refs.team_id,
                        "membership_user_id": refs.membership_user_id,
                        "organization_id": refs.organization_id,
                        "project_id": refs.project_id,
                    },
                    "entries": entries,
                })
            })
            .collect(),
    )
}

fn identity_keys(calls: &[CacheKeyCall]) -> Value {
    Value::Array(
        calls
            .iter()
            .map(|call| {
                json!(identity_cache_keys(
                    &call.api_key,
                    call.end_user_id.as_deref(),
                    call.key_is_resolved,
                ))
            })
            .collect(),
    )
}

fn row_validation(rows: &[RowCall], refreshed_at: f64) -> Value {
    Value::Array(
        rows.iter()
            .map(|call| {
                let object = validate_row(call.row, call.value.as_ref(), refreshed_at);
                json!({"validated": object.is_some(), "stamp": stamp(object.as_ref())})
            })
            .collect(),
    )
}

fn stamp(object: Option<&RowObject>) -> Option<f64> {
    match object {
        Some(RowObject::Team(row) | RowObject::Project(row)) => row.last_refreshed_at,
        Some(RowObject::User(_) | RowObject::Membership(_) | RowObject::Organization(_)) | None => {
            None
        }
    }
}

fn identity_ttls(ttls: &[TtlCall]) -> Value {
    Value::Array(
        ttls.iter()
            .map(|call| json!(identity_memory_ttl(&call.value, call.management_ttl)))
            .collect(),
    )
}
