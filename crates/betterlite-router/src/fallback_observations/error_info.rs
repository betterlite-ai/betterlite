use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::class_message;
use betterlite_error::error::metadata::ErrorMetadata;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::{Deserialize, Serialize};

// add_retry_fallback_headers' own name for the header that repeats a response's fallback errors
pub const FALLBACK_ERRORS_HEADER: &str = "x-litellm-fallback-errors";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackErrorInfo {
    pub message: String,
    // LiteLLM's TypedDict names this field type, which is a keyword here
    #[serde(rename = "type")]
    pub error_type: String,
    pub param: Option<String>,
    pub code: Option<String>,
}

// get_fallback_error_info: message, type, param and status_code read off the exception, with a class-name and None fallback
#[must_use]
pub fn fallback_error_info(class: &ErrorClass) -> FallbackErrorInfo {
    let metadata = class.metadata();
    let fields = family_fields(class);
    FallbackErrorInfo {
        message: fields.message,
        error_type: error_type(&metadata, fields.missing_type_name),
        param: optional_text(&metadata.param),
        code: code_text(class, &metadata),
    }
}

// the two strings get_fallback_error_info reads off an exception family: the text its Python constructor built, and the class name it reports when the exception has no type attribute
pub struct FamilyFields {
    pub message: String,
    // an exception outside every named family reaches the router as the plain Exception Python raised
    pub missing_type_name: &'static str,
}

#[must_use]
pub fn family_fields(class: &ErrorClass) -> FamilyFields {
    FamilyFields {
        message: class_message(class),
        missing_type_name: missing_type_name(class),
    }
}

// the class name getattr(error, "type", error.__class__.__name__) reports when the exception carries no type attribute
const fn missing_type_name(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::GuardrailRaised(_) => "GuardrailRaisedException",
        ErrorClass::BudgetExceeded(_) => "BudgetExceededError",
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError",
        ErrorClass::Authentication(_)
        | ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::BadRequest(_)
        | ErrorClass::ContextWindowExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::Timeout(_)
        | ErrorClass::RateLimit(_)
        | ErrorClass::ServiceUnavailable(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::Unclassified(_)
        | ErrorClass::Api(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::JsonSchemaValidation(_) => "Exception",
    }
}

// getattr(error, "type", error.__class__.__name__): a null type attribute reads as the string None
fn error_type(metadata: &ErrorMetadata, missing_type_name: &'static str) -> String {
    match &metadata.error_type {
        WireField::Text(text) => text.clone(),
        WireField::Null => "None".to_owned(),
        WireField::Missing(_) => missing_type_name.to_owned(),
    }
}

fn optional_text(field: &WireField) -> Option<String> {
    match field {
        WireField::Text(text) => Some(text.clone()),
        WireField::Null | WireField::Missing(_) => None,
    }
}

// getattr(error, "status_code", getattr(error, "code", None)): a plain Exception has neither, so its code stays null
fn code_text(class: &ErrorClass, metadata: &ErrorMetadata) -> Option<String> {
    if matches!(class, ErrorClass::Unclassified(_)) {
        return None;
    }
    Some(metadata.status_code.to_string())
}

// get_fallback_errors_from_headers: a list keeps its object entries, a JSON string parses, anything else contributes nothing
#[must_use]
pub fn fallback_errors_from_headers(additional_headers: &AdditionalHeaders) -> Vec<JsonObject> {
    let Some(existing) = additional_headers.get(FALLBACK_ERRORS_HEADER) else {
        return Vec::new();
    };
    match existing {
        Json::List(items) => object_entries(items),
        Json::Text(text) => parsed_object_entries(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => Vec::new(),
    }
}

fn object_entries(items: &[Json]) -> Vec<JsonObject> {
    items
        .iter()
        .filter_map(|item| match item {
            Json::Object(object) => Some(object.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

// json.loads's JSONDecodeError drops the whole header, and a parsed value that is not a list contributes nothing either
fn parsed_object_entries(text: &str) -> Vec<JsonObject> {
    match serde_json::from_str::<Json>(text) {
        Ok(Json::List(items)) => object_entries(&items),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | Err(_) => Vec::new(),
    }
}

// FallbackErrorInfo's wire shape in the field order LiteLLM's TypedDict was built with
#[must_use]
pub fn error_object(error: &FallbackErrorInfo) -> JsonObject {
    let param = error
        .param
        .as_deref()
        .map_or_else(|| Json::Null, |text| Json::Text(text.to_owned()));
    let code = error
        .code
        .as_deref()
        .map_or_else(|| Json::Null, |text| Json::Text(text.to_owned()));
    JsonObject::from_iter([
        ("message".to_owned(), Json::Text(error.message.clone())),
        ("type".to_owned(), Json::Text(error.error_type.clone())),
        ("param".to_owned(), param),
        ("code".to_owned(), code),
    ])
}
