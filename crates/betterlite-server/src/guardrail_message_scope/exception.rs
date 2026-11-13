use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

mod tests;

// Whether the HTTP framework LiteLLM imports lazily is present; without it every rejection is a GuardrailRaisedException.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastapiInstall {
    Installed,
    Absent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpException {
    pub status_code: u16,
    pub detail: GuardrailValue,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct GuardrailRaisedException {
    pub guardrail_name: Option<String>,
    pub message: String,
    pub status_code: u16,
    pub blocked_content: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RaisedError {
    Http(HttpException),
    Guardrail(GuardrailRaisedException),
    Other,
}

// The two fields of a guardrail callback that an error body can carry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GuardrailCallback {
    pub guardrail_name: Option<String>,
    pub event_hook: Option<GuardrailValue>,
}

#[must_use]
pub fn is_fastapi_http_exception(
    error: &RaisedError,
    block_status_codes: &[u16],
    fastapi: FastapiInstall,
) -> bool {
    let RaisedError::Http(http) = error else {
        return false;
    };
    fastapi == FastapiInstall::Installed && block_status_codes.contains(&http.status_code)
}

const fn is_unset(value: &GuardrailValue) -> bool {
    match value {
        GuardrailValue::Null => true,
        GuardrailValue::Text(text) => text.is_empty(),
        GuardrailValue::Array(items) => items.is_empty(),
        GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Object(_) => false,
    }
}

// Only an existing detail object is touched, and a key the error already carries is never overwritten.
pub fn add_guardrail_context(
    error: &mut RaisedError,
    callback: &GuardrailCallback,
    fastapi: FastapiInstall,
) {
    let RaisedError::Http(HttpException {
        detail: GuardrailValue::Object(detail),
        ..
    }) = error
    else {
        return;
    };
    if fastapi == FastapiInstall::Absent {
        return;
    }
    if let Some(name) = callback
        .guardrail_name
        .as_ref()
        .filter(|name| !name.is_empty())
    {
        detail
            .entry("guardrail_name".to_owned())
            .or_insert_with(|| GuardrailValue::Text(name.clone()));
    }
    if let Some(hook) = callback.event_hook.as_ref().filter(|hook| !is_unset(hook)) {
        detail
            .entry("guardrail_mode".to_owned())
            .or_insert_with(|| hook.clone());
    }
}

#[must_use]
pub fn pre_call_rejection(
    message: &str,
    guardrail_name: Option<&str>,
    fastapi: FastapiInstall,
) -> RaisedError {
    match fastapi {
        FastapiInstall::Installed => RaisedError::Http(HttpException {
            status_code: 400,
            detail: GuardrailValue::Object(Object::from([(
                "error".to_owned(),
                GuardrailValue::Text(message.to_owned()),
            )])),
        }),
        FastapiInstall::Absent => RaisedError::Guardrail(GuardrailRaisedException {
            guardrail_name: guardrail_name.map(str::to_owned),
            message: message.to_owned(),
            status_code: 400,
            blocked_content: false,
        }),
    }
}
