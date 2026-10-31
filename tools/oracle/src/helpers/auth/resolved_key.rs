use std::collections::BTreeMap;

use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_auth::resolved_key::view::VerificationTokenView;
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
            Function::ServiceAccount => predicate(&input.value, factories::is_team_service_account),
            Function::AdminView => predicate(&input.value, factories::user_api_key_has_admin_view),
        }
    }
}

fn role(value: &Value) -> HelperResult {
    let decoded: UserRole = serde_json::from_value(value.clone())
        .map_err(|error| to_error("ValueError", error.to_string()))?;
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
    let decoded = VerificationTokenView::construct(fields)
        .map_err(|error| to_error("ValidationError", error.to_string()))?;
    serde_json::to_value(decoded).map_err(|error| encoding_error(&error))
}

fn auth(value: &Value) -> HelperResult {
    let fields = object_fields(value)?;
    let decoded =
        UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(|error| auth_error(&error))?;
    serde_json::to_value(decoded).map_err(|error| encoding_error(&error))
}

fn hash_token(value: &Value) -> HelperResult {
    let text = optional_string_value(value)?.map_or_else(
        || Err(to_error("ValueError", "value must be a string".to_owned())),
        Ok,
    )?;
    let hashed = credential::hash_token(&text);
    Ok(serde_json::json!({ "hash": hashed }))
}

fn is_jwt(value: &Value) -> HelperResult {
    let token = optional_string_value(value)?;
    let shaped = credential::is_jwt(token.as_deref());
    Ok(serde_json::json!({ "is_jwt": shaped }))
}

fn principal(constructed: Result<UserAPIKeyAuth, AuthError>) -> HelperResult {
    let decoded = constructed.map_err(|error| auth_error(&error))?;
    let encoded = serde_json::to_value(decoded).map_err(|error| encoding_error(&error))?;
    Ok(encoded)
}

fn predicate(value: &Value, check: fn(&UserAPIKeyAuth) -> bool) -> HelperResult {
    let fields = object_fields(value)?;
    let decoded =
        UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(|error| auth_error(&error))?;
    Ok(serde_json::json!({ "result": check(&decoded) }))
}

fn object_fields(value: &Value) -> Result<BTreeMap<String, JsonData>, Box<HelperError>> {
    let decoded: JsonData =
        serde_json::from_value(value.clone()).map_err(|error| encoding_error(&error))?;
    match decoded {
        JsonData::Object(fields) => Ok(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(to_error(
            "ValueError",
            "value must be a JSON object".to_owned(),
        )),
    }
}

fn optional_string_value(value: &Value) -> Result<Option<String>, Box<HelperError>> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => Err(to_error(
            "ValueError",
            "value must be a string or null".to_owned(),
        )),
    }
}

fn auth_error(error: &AuthError) -> Box<HelperError> {
    let class = match error {
        AuthError::NonStringApiKey => "TypeError",
        AuthError::View(_) | AuthError::Json(_) => "ValidationError",
    };
    to_error(class, error.to_string())
}

fn encoding_error(error: &serde_json::Error) -> Box<HelperError> {
    let message = format!("resolved key payload could not be encoded: {error}");
    to_error("ValueError", message)
}

fn to_error(class: &str, message: String) -> Box<HelperError> {
    let owned_class = class.to_owned();
    Box::new(HelperError {
        class: owned_class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
