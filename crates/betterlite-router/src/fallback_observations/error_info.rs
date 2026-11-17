use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::{BudgetExceededDetails, SchemaValidationDetails};
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
    FallbackErrorInfo {
        message: exception_message(class),
        error_type: error_type(class, &metadata),
        param: optional_text(&metadata.param),
        code: code_text(class, &metadata),
    }
}

// getattr(error, "message", str(error)): every family carries the text its Python constructor built
#[must_use]
pub fn exception_message(class: &ErrorClass) -> String {
    match class {
        ErrorClass::Authentication(details) => details.message.clone(),
        ErrorClass::NotFound(details) => details.message.clone(),
        ErrorClass::PermissionDenied(details) => details.message.clone(),
        ErrorClass::UnprocessableEntity(details) => details.message.clone(),
        ErrorClass::BadRequest(details)
        | ErrorClass::ContextWindowExceeded(details)
        | ErrorClass::ContentPolicyViolation(details) => details.message.clone(),
        ErrorClass::Timeout(details) => details.context.message.clone(),
        ErrorClass::RateLimit(details) => details.context.message.clone(),
        ErrorClass::BudgetExceeded(details) => budget_message(details),
        ErrorClass::ServiceUnavailable(context)
        | ErrorClass::BadGateway(context)
        | ErrorClass::InternalServer(context)
        | ErrorClass::ApiConnection(context)
        | ErrorClass::ApiResponseValidation(context)
        | ErrorClass::UnsupportedParams(context)
        | ErrorClass::Unclassified(context) => context.message.clone(),
        ErrorClass::Api(details) => details.context.message.clone(),
        ErrorClass::MidStreamFallback(details) => details.context.message.clone(),
        ErrorClass::JsonSchemaValidation(details) => schema_message(details),
        ErrorClass::GuardrailRaised(details) => details.message.clone(),
    }
}

// getattr(error, "type", error.__class__.__name__): a null type attribute reads as the string None
fn error_type(class: &ErrorClass, metadata: &ErrorMetadata) -> String {
    match &metadata.error_type {
        WireField::Text(text) => text.clone(),
        WireField::Null => "None".to_owned(),
        WireField::Missing(_) => missing_type_name(class),
    }
}

// the families without a type attribute, whose class name LiteLLM reads instead; an exception outside every family reaches the router as the plain Exception Python raised
fn missing_type_name(class: &ErrorClass) -> String {
    match class {
        ErrorClass::BudgetExceeded(_) => "BudgetExceededError".to_owned(),
        ErrorClass::GuardrailRaised(_) => "GuardrailRaisedException".to_owned(),
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError".to_owned(),
        ErrorClass::Authentication(_)
        | ErrorClass::NotFound(_)
        | ErrorClass::PermissionDenied(_)
        | ErrorClass::BadRequest(_)
        | ErrorClass::UnprocessableEntity(_)
        | ErrorClass::Timeout(_)
        | ErrorClass::RateLimit(_)
        | ErrorClass::ContextWindowExceeded(_)
        | ErrorClass::ContentPolicyViolation(_)
        | ErrorClass::ServiceUnavailable(_)
        | ErrorClass::BadGateway(_)
        | ErrorClass::InternalServer(_)
        | ErrorClass::Api(_)
        | ErrorClass::ApiConnection(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => "Exception".to_owned(),
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

// BudgetExceededError.__init__ replaces an absent or empty message with the costs it was given
fn budget_message(details: &BudgetExceededDetails) -> String {
    match details
        .message
        .as_deref()
        .filter(|message| !message.is_empty())
    {
        Some(message) => message.to_owned(),
        None => format!(
            "Budget has been exceeded! Current cost: {}, Max budget: {}",
            details.current_cost, details.max_budget
        ),
    }
}

// JSONSchemaValidationError.__init__ builds its message from the identity and the response it validated
fn schema_message(details: &SchemaValidationDetails) -> String {
    format!(
        "litellm.JSONSchemaValidationError: model={}, returned an invalid response={}, for schema={}.\nAccess raw response with `e.raw_response`",
        details.model, details.raw_response, details.schema
    )
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
    JsonObject::from_iter([
        ("message".to_owned(), Json::Text(error.message.clone())),
        ("type".to_owned(), Json::Text(error.error_type.clone())),
        ("param".to_owned(), optional_json(error.param.as_deref())),
        ("code".to_owned(), optional_json(error.code.as_deref())),
    ])
}

fn optional_json(value: Option<&str>) -> Json {
    match value {
        Some(text) => Json::Text(text.to_owned()),
        None => Json::Null,
    }
}
