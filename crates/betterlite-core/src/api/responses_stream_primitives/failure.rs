use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::error::ErrorClass;
use betterlite_logging::analytics::python::repr;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;

use super::error_fields::{
    StreamErrorFields, StreamErrorObject, error_event_fields, status_code_for_error_fields,
};

// mirrors BaseLLMException(status_code, message, body) handed to litellm.exception_type
#[must_use]
pub fn map_stream_error_to_exception(
    object: StreamErrorObject<'_>,
    model: &str,
    custom_llm_provider: &str,
    redactor: &Redactor,
) -> ErrorClass {
    let fields = error_event_fields(object);
    let status_code =
        status_code_for_error_fields(fields.error_type.as_deref(), fields.code.as_deref());
    let message = provider_error_message(&fields, status_code);
    let raw = RawException {
        model: model.to_owned(),
        custom_llm_provider: custom_llm_provider.to_owned(),
        error_str: message.clone(),
        status_code: Some(status_code),
        status_code_is_synthesized: false,
        message: Some(message),
        body: Some(RawExceptionBody {
            kind: fields.error_type.clone(),
            param: None,
            code: fields.code.clone(),
            message: Some(fields.message),
            error: None,
        }),
        exception_type: "BaseLLMException".to_owned(),
    };
    dispatch(&DispatchInput::Raw(Box::new(raw)), redactor)
}

// the source's f-string embeds str(body), so the message carries Python's dict repr, not JSON
#[must_use]
pub fn provider_error_message(fields: &StreamErrorFields, status_code: u16) -> String {
    let body = error_body_text(fields);
    format!("Error code: {status_code} - {{'error': {body}}}")
}

// str() of the error dict keeps the source's insertion order: message, type, code
fn error_body_text(fields: &StreamErrorFields) -> String {
    let mut body = IndexMap::new();
    body.insert("message".to_owned(), Json::Text(fields.message.clone()));
    body.insert(
        "type".to_owned(),
        fields.error_type.clone().map_or(Json::Null, Json::Text),
    );
    body.insert(
        "code".to_owned(),
        fields.code.clone().map_or(Json::Null, Json::Text),
    );
    // the formatter writes into a Vec, so its io error branch is unreachable
    repr(&Json::Object(body)).unwrap_or_default()
}

// getattr(mapped_exception, "status_code", None) with a missing or non-integer status read as None
#[must_use]
pub fn mid_stream_fallback_eligible(class: &ErrorClass, status_code: Option<u16>) -> bool {
    matches!(class, ErrorClass::ContentPolicyViolation(_))
        || status_code.is_none_or(|status| status >= 500 || status == 429)
}
