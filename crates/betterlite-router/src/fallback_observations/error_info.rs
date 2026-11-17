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
    match class {
        ErrorClass::Authentication(details) => FamilyFields {
            message: details.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::NotFound(details) => FamilyFields {
            message: details.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::PermissionDenied(details) => FamilyFields {
            message: details.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::UnprocessableEntity(details) => FamilyFields {
            message: details.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::BadRequest(details)
        | ErrorClass::ContextWindowExceeded(details)
        | ErrorClass::ContentPolicyViolation(details) => FamilyFields {
            message: details.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::GuardrailRaised(details) => FamilyFields {
            message: details.message.clone(),
            missing_type_name: "GuardrailRaisedException",
        },
        ErrorClass::Timeout(details) => FamilyFields {
            message: details.context.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::RateLimit(details) => FamilyFields {
            message: details.context.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::BudgetExceeded(details) => FamilyFields {
            message: budget_message(details),
            missing_type_name: "BudgetExceededError",
        },
        ErrorClass::ServiceUnavailable(context)
        | ErrorClass::BadGateway(context)
        | ErrorClass::InternalServer(context)
        | ErrorClass::ApiConnection(context)
        | ErrorClass::ApiResponseValidation(context)
        | ErrorClass::Unclassified(context) => FamilyFields {
            message: context.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::UnsupportedParams(context) => FamilyFields {
            message: context.message.clone(),
            missing_type_name: "UnsupportedParamsError",
        },
        ErrorClass::Api(details) => FamilyFields {
            message: details.context.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::MidStreamFallback(details) => FamilyFields {
            message: details.context.message.clone(),
            missing_type_name: "Exception",
        },
        ErrorClass::JsonSchemaValidation(details) => FamilyFields {
            message: schema_message(details),
            missing_type_name: "Exception",
        },
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

// BudgetExceededError.__init__ replaces an absent or empty message with the costs it was given
fn budget_message(details: &BudgetExceededDetails) -> String {
    details
        .message
        .as_deref()
        .filter(|message| !message.is_empty())
        .map_or_else(
            || {
                format!(
                    "Budget has been exceeded! Current cost: {}, Max budget: {}",
                    details.current_cost, details.max_budget
                )
            },
            str::to_owned,
        )
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
