use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{
    AuthError, AuthInput, UserAPIKeyAuth, is_team_service_account, user_api_key_has_admin_view,
};
use betterlite_auth::resolved_key::view::{VerificationTokenView, ViewError};
use betterlite_auth::resolved_key::{credential, factories};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{HelperError, HelperResult};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Role,
    View,
    Auth,
    HashToken,
    IsJwt,
    HealthPrincipal,
    JobsPrincipal,
    ServiceAccount,
    AdminView,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    value: Value,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Role => role(&input.value),
            Function::View => view(&input.value),
            Function::Auth => auth(&input.value),
            Function::HashToken => hash_token(&input.value),
            Function::IsJwt => is_jwt(&input.value),
            Function::HealthPrincipal => principal(factories::system_principal(
                &factories::SystemPrincipal::HealthCheck,
            )),
            Function::JobsPrincipal => principal(factories::system_principal(
                &factories::SystemPrincipal::Jobs,
            )),
            Function::ServiceAccount => predicate(&input.value, is_team_service_account),
            Function::AdminView => predicate(&input.value, user_api_key_has_admin_view),
        }
    }
}

enum Failure {
    InvalidRole(serde_json::Error),
    InvalidValue(&'static str),
    Encoding(serde_json::Error),
    View(ViewError),
    Auth(AuthError),
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let (class, message) = match failure {
            Failure::InvalidRole(error) => ("ValueError", error.to_string()),
            Failure::InvalidValue(message) => ("ValueError", message.to_owned()),
            Failure::Encoding(error) => (
                "ValueError",
                format!("resolved key payload could not be encoded: {error}"),
            ),
            Failure::View(error) => ("ValidationError", error.to_string()),
            Failure::Auth(error) => {
                let class = match &error {
                    AuthError::NonStringApiKey => "TypeError",
                    AuthError::View(_) | AuthError::Json(_) => "ValidationError",
                };
                (class, error.to_string())
            }
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message,
        })
    }
}

fn role(value: &Value) -> HelperResult {
    let decoded: UserRole = serde_json::from_value(value.clone()).map_err(Failure::InvalidRole)?;
    let (description, ui_label) = decoded.description_and_ui_label();
    Ok(serde_json::json!({
        "value": decoded.to_string(),
        "description": description,
        "ui_label": ui_label,
        "is_internal_user_role": decoded.is_internal_user_role(),
    }))
}

fn view(value: &Value) -> HelperResult {
    let fields = object_fields(value)?;
    let decoded = VerificationTokenView::construct(fields).map_err(Failure::View)?;
    Ok(serde_json::to_value(decoded).map_err(Failure::Encoding)?)
}

fn auth(value: &Value) -> HelperResult {
    let fields = object_fields(value)?;
    let decoded = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(Failure::Auth)?;
    Ok(serde_json::to_value(decoded).map_err(Failure::Encoding)?)
}

fn hash_token(value: &Value) -> HelperResult {
    let Some(text) = optional_string_value(value)? else {
        return Err(Failure::InvalidValue("value must be a string").into());
    };
    let hashed = credential::hash_token(&text);
    Ok(serde_json::json!({ "hash": hashed }))
}

fn is_jwt(value: &Value) -> HelperResult {
    let token = optional_string_value(value)?;
    let shaped = credential::is_jwt(token.as_deref());
    Ok(serde_json::json!({ "is_jwt": shaped }))
}

fn principal(constructed: Result<UserAPIKeyAuth, AuthError>) -> HelperResult {
    let decoded = constructed.map_err(Failure::Auth)?;
    Ok(serde_json::to_value(decoded).map_err(Failure::Encoding)?)
}

fn predicate(value: &Value, check: fn(&UserAPIKeyAuth) -> bool) -> HelperResult {
    let fields = object_fields(value)?;
    let decoded = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(Failure::Auth)?;
    Ok(serde_json::json!({ "result": check(&decoded) }))
}

fn object_fields(value: &Value) -> Result<BTreeMap<String, JsonData>, Box<HelperError>> {
    let decoded: JsonData = serde_json::from_value(value.clone()).map_err(Failure::Encoding)?;
    match decoded {
        JsonData::Object(fields) => Ok(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(Failure::InvalidValue("value must be a JSON object").into()),
    }
}

fn optional_string_value(value: &Value) -> Result<Option<String>, Box<HelperError>> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            Err(Failure::InvalidValue("value must be a string or null").into())
        }
    }
}
