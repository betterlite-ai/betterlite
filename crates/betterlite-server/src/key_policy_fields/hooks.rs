use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_pyhost::call_error::CallError;
use betterlite_pyhost::json_values::{JsonObject, JsonValue};

use crate::key_fields::decode::{DecodeError, to_json};
use crate::key_fields::policy::{CustomKeyPolicyRequest, PolicyRequestBody};
use crate::key_fields::update::UpdateKeyRequest;
use crate::python_hooks::slots::LoadedHook;

pub const UPDATE_NOT_COROUTINE: &str = "user_custom_key_update must be a coroutine";
pub const POLICY_NOT_COROUTINE: &str = "user_custom_key_policy must be a coroutine";
pub const DEFAULT_DENIAL: &str = "Authentication Failed - Custom Auth Rule";

// the three configured callables that B165 loads at startup; a missing setting is a missing hook
#[derive(Default)]
pub struct KeyHooks {
    pub generate: Option<LoadedHook>,
    pub update: Option<LoadedHook>,
    pub policy: Option<LoadedHook>,
}

#[derive(Debug, thiserror::Error)]
pub enum HookPolicyError {
    #[error("{message}")]
    NotCoroutine { message: &'static str },
    #[error(transparent)]
    Call(#[from] Box<CallError>),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("the custom key hook returned a value that is not a mapping")]
    ResultNotAnObject,
    #[error("{field} must be a boolean")]
    DecisionNotBool { field: &'static str },
    #[error("{field} must be a string")]
    MessageNotString { field: &'static str },
    #[error("{message}")]
    Denied { message: String },
}

impl HookPolicyError {
    // only the denial is an HTTPException; every other failure reaches the caller as itself
    #[must_use]
    pub const fn status_code(&self) -> Option<StatusCode> {
        if matches!(self, Self::Denied { .. }) {
            return Some(StatusCode::FORBIDDEN);
        }
        None
    }
}

pub async fn enforce_key_update_hook(
    hook: Option<&LoadedHook>,
    data: &UpdateKeyRequest,
) -> Result<(), HookPolicyError> {
    let Some(hook) = hook else {
        return Ok(());
    };
    ensure_coroutine(hook, UPDATE_NOT_COROUTINE)?;
    let argument = request_payload(data)?;
    evaluate(hook, argument).await
}

// the policy request is built only after the hook proved to be a coroutine, exactly as LiteLLM orders it
pub async fn enforce_key_policy_hook<Build>(
    hook: Option<&LoadedHook>,
    build_policy_request: Build,
) -> Result<(), HookPolicyError>
where
    Build: FnOnce() -> Result<CustomKeyPolicyRequest, DecodeError>,
{
    let Some(hook) = hook else {
        return Ok(());
    };
    ensure_coroutine(hook, POLICY_NOT_COROUTINE)?;
    let request = policy_payload(&build_policy_request()?)?;
    evaluate(hook, request).await
}

fn ensure_coroutine(hook: &LoadedHook, message: &'static str) -> Result<(), HookPolicyError> {
    let handle = hook.object.loaded.handle(&hook.host).map_err(Box::new)?;
    if handle.is_async {
        return Ok(());
    }
    Err(HookPolicyError::NotCoroutine { message })
}

async fn evaluate(hook: &LoadedHook, argument: JsonValue) -> Result<(), HookPolicyError> {
    let result = hook
        .call(vec![argument], JsonObject::new())
        .await
        .map_err(Box::new)?;
    let JsonValue::Object(fields) = &result else {
        return Err(HookPolicyError::ResultNotAnObject);
    };
    let decision = match fields.get("decision") {
        None | Some(JsonValue::Null) => true,
        Some(JsonValue::Bool(flag)) => *flag,
        Some(_) => return Err(HookPolicyError::DecisionNotBool { field: "decision" }),
    };
    if decision {
        return Ok(());
    }
    let message = match fields.get("message") {
        None | Some(JsonValue::Null) => DEFAULT_DENIAL.to_owned(),
        Some(JsonValue::String(text)) => text.clone(),
        Some(_) => return Err(HookPolicyError::MessageNotString { field: "message" }),
    };
    Err(HookPolicyError::Denied { message })
}

// the hook receives the request model's JSON dump, defaults filled, as the pydantic object would print
fn request_payload(data: &UpdateKeyRequest) -> Result<JsonValue, HookPolicyError> {
    serde_json::to_value(data.project()?.resolved)
        .map_err(|source| HookPolicyError::Decode(DecodeError::Document(source)))
}

fn policy_payload(request: &CustomKeyPolicyRequest) -> Result<JsonValue, HookPolicyError> {
    let existing_key = match &request.existing_key {
        Some(key) => to_json(key)?,
        None => JsonData::Null,
    };
    let body = match &request.request {
        PolicyRequestBody::Generate(inner) => inner.project()?.resolved,
        PolicyRequestBody::Update(inner) => inner.project()?.resolved,
        PolicyRequestBody::Regenerate(inner) => inner.project()?.resolved,
    };
    Ok(serde_json::json!({
        "operation": &request.operation,
        "existing_key": existing_key,
        "effective_key": &request.effective_key,
        "request": body,
    }))
}
