use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::{CredentialRef, NetworkContext};
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

mod carrier;
mod error;
mod principal;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "aspect", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Principal(Box<PrincipalInput>),
    Carrier(Box<CarrierInput>),
    Error(error::ErrorInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrincipalInput {
    key: Value,
    auth_method: AuthMethod,
    #[serde(default)]
    issuer: Option<String>,
    #[serde(default)]
    subject_fallback: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    credential_ref: Option<CredentialRef>,
    #[serde(default)]
    network: Option<NetworkContext>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CarrierInput {
    key: Value,
    keep_source_key: bool,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Principal(fields) => principal::principal(fields),
            Input::Carrier(fields) => carrier::carrier(fields),
            Input::Error(fields) => Ok(error::error(fields)),
        }
    }
}

// UserAPIKeyAuth.model_validate reads a null optional field as absent
fn auth_from_fields(value: &Value) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut fields: BTreeMap<String, JsonData> =
        serde_json::from_value(value.clone()).or_value_error()?;
    fields.retain(|_, field| !matches!(field, JsonData::Null));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

fn key_fields(key: &UserAPIKeyAuth) -> Result<Value, Box<HelperError>> {
    let view = serde_json::to_value(key).or_value_error()?;
    Ok(json!({
        "token": view.get("token"),
        "key_name": view.get("key_name"),
        "user_id": view.get("user_id"),
        "team_id": view.get("team_id"),
        "has_object_permission": view
            .get("object_permission")
            .is_some_and(|permission| !permission.is_null()),
    }))
}
