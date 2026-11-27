use std::collections::BTreeMap;

use betterlite_auth::auth_key_lookup::jwt::jwt_key_mapping_cache_key;
use betterlite_auth::auth_key_lookup::key_identity;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

mod fixtures;
mod key_object_answer;
mod mapping_answer;
mod permissions;
mod session_answer;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "aspect", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    CacheKey(CacheKeyInput),
    Mapping(Box<MappingInput>),
    KeyObject(Box<KeyObjectInput>),
    Session(Box<SessionInput>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CacheKeyInput {
    claim_name: String,
    claim_value: String,
    issuer: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingInput {
    claim_name: String,
    claim_value: String,
    issuer: Option<String>,
    #[serde(default)]
    rows: Vec<MappingRow>,
    lookup: MappingLookup,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum MappingLookup {
    Token {
        hashed_token: String,
    },
    Tokens {
        hashed_tokens: Vec<String>,
    },
    Object {
        #[serde(default)]
        found: Option<MappingRow>,
    },
}

#[derive(Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingRow {
    token: String,
    jwt_claim_name: String,
    jwt_claim_value: String,
    #[serde(default)]
    jwt_issuer: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyObjectInput {
    hashed_token: String,
    #[serde(default)]
    cached: Option<Value>,
    #[serde(default)]
    rows: Vec<RowStep>,
    reconnect: ReconnectInput,
    #[serde(default)]
    permissions: Option<PermissionInput>,
    #[serde(default)]
    check_db_only: bool,
    #[serde(default)]
    check_cache_only: Option<bool>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RowStep {
    Row { fields: Value },
    Transport,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReconnectInput {
    #[serde(default)]
    answer: bool,
    #[serde(default)]
    timeout: Option<f64>,
    #[serde(default)]
    lock: Option<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionInput {
    id: String,
    #[serde(default)]
    fail: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionInput {
    auth: Value,
    #[serde(default)]
    user: Option<Value>,
    #[serde(default)]
    user_outcome: UserOutcome,
    #[serde(default)]
    team: Option<Value>,
    #[serde(default)]
    membership: Option<Value>,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UserOutcome {
    #[default]
    Missing,
    Gone,
    Outage,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        runtime.block_on(answer(input))
    }
}

async fn answer(input: &Input) -> HelperResult {
    match input {
        Input::CacheKey(fields) => Ok(json!({
            "key": jwt_key_mapping_cache_key(
                &fields.claim_name,
                &fields.claim_value,
                fields.issuer.as_deref(),
            ),
        })),
        Input::Mapping(fields) => mapping_answer::mapping(fields).await,
        Input::KeyObject(fields) => key_object_answer::key_object(fields).await,
        Input::Session(fields) => session_answer::session(fields).await,
    }
}

fn token_fields(auth: &UserAPIKeyAuth) -> Result<Value, Box<HelperError>> {
    let identity = key_identity(auth);
    let value = serde_json::to_value(auth).or_value_error()?;
    Ok(json!({
        "key_name": identity.key_name,
        "user_id": identity.user_id,
        "team_id": identity.team_id,
        "team_alias": identity.team_alias,
        "user_role": identity.user_role,
        "is_session_token": identity.is_session_token,
        "models": value.get("models").cloned().unwrap_or_else(|| json!([])),
        "team_models": value.get("team_models").cloned().unwrap_or_else(|| json!([])),
        "object_permission_id": value.get("object_permission_id").cloned().unwrap_or(Value::Null),
        "has_object_permission": value
            .get("object_permission")
            .is_some_and(|permission| !permission.is_null()),
    }))
}

fn auth_from_fields(value: &Value) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut fields: BTreeMap<String, JsonData> =
        serde_json::from_value(value.clone()).or_value_error()?;
    // UserAPIKeyAuth.model_validate reads a null optional field as absent
    fields.retain(|_, field| !matches!(field, JsonData::Null));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}
