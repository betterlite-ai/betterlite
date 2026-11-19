use betterlite_azure::azure_assistant_definitions::error::DefinitionsError;
use betterlite_error::dispatch::{DispatchInput, RawException, RawExceptionBody, dispatch};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_redact::secret::Redactor;
use serde_json::Value;

use betterlite_openai::assistants::error::AssistantsError;

// The SDK's own message when the client is built without a key; exception_type maps its prefix to an auth failure.
const MISSING_KEY: &str = "The api_key client option must be set either by passing api_key \
to the client or by setting the OPENAI_API_KEY environment variable";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Sync,
    Async,
}

// What one create_assistants call can fail with before the provider's error projection runs.
pub enum Raised {
    Local(String),
    Assistants(AssistantsError),
    Definitions(DefinitionsError),
}

// The exception the LiteLLM façade raises: its class name, status and the type/param/code attributes it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantCreateFailure {
    pub class: String,
    pub status: Option<u16>,
    pub kind: WireField,
    pub param: WireField,
    pub code: WireField,
    pub message: String,
}

pub fn unsupported(provider: &str) -> AssistantCreateFailure {
    AssistantCreateFailure {
        class: String::from("BadRequestError"),
        status: Some(400),
        kind: WireField::Null,
        param: WireField::Null,
        code: WireField::Null,
        message: format!(
            "LiteLLM doesn't support {provider} for 'create_assistants'. Only 'openai' is supported."
        ),
    }
}

pub fn no_response() -> AssistantCreateFailure {
    AssistantCreateFailure {
        class: String::from("InternalServerError"),
        status: Some(500),
        kind: WireField::Text(String::from("internal_server_error")),
        param: WireField::Null,
        code: WireField::Null,
        message: String::from("No response returned from 'create_assistants'"),
    }
}

// acreate_assistants wraps every failure in exception_type; create_assistants lets the SDK's own exception out
pub fn raised(
    raised: Raised,
    model: &str,
    provider: &str,
    entry: Entry,
    redactor: &Redactor,
) -> AssistantCreateFailure {
    let (class, status, message) = source(&raised);
    match entry {
        Entry::Sync => {
            let (kind, param, code) = sdk_fields(&raised);
            AssistantCreateFailure {
                class,
                status,
                kind,
                param,
                code,
                message,
            }
        }
        Entry::Async => wrapped(&class, status, &message, &raised, model, provider, redactor),
    }
}

// openai.APIError's subclasses read type/param/code off the flattened body; every other exception carries none
fn sdk_fields(raised: &Raised) -> (WireField, WireField, WireField) {
    match raised {
        Raised::Assistants(AssistantsError::Status { body, .. })
        | Raised::Definitions(DefinitionsError::Assistants(AssistantsError::Status {
            body, ..
        })) => body_fields(body),
        Raised::Assistants(
            AssistantsError::Transport { .. }
            | AssistantsError::Malformed { .. }
            | AssistantsError::Truncated { .. }
            | AssistantsError::Shape { .. },
        )
        | Raised::Definitions(DefinitionsError::Assistants(
            AssistantsError::Transport { .. }
            | AssistantsError::Malformed { .. }
            | AssistantsError::Truncated { .. }
            | AssistantsError::Shape { .. },
        )) => (WireField::Null, WireField::Null, WireField::Null),
        Raised::Local(_) | Raised::Assistants(_) | Raised::Definitions(_) => {
            (WireField::MISSING, WireField::MISSING, WireField::MISSING)
        }
    }
}

fn body_fields(body: &[u8]) -> (WireField, WireField, WireField) {
    match flattened(body) {
        Some(fields) => (
            fields.kind.map_or(WireField::Null, WireField::Text),
            fields.param.map_or(WireField::Null, WireField::Text),
            fields.code.map_or(WireField::Null, WireField::Text),
        ),
        None => (WireField::Null, WireField::Null, WireField::Null),
    }
}

fn source(raised: &Raised) -> (String, Option<u16>, String) {
    match raised {
        Raised::Local(message) => (String::from("ValueError"), None, message.clone()),
        Raised::Assistants(error) => assistants(error),
        Raised::Definitions(error) => definitions(error),
    }
}

fn assistants(error: &AssistantsError) -> (String, Option<u16>, String) {
    match error {
        AssistantsError::Status { status, body } => (
            sdk_class(*status).to_owned(),
            Some(*status),
            format!("Error code: {status} - {}", String::from_utf8_lossy(body)),
        ),
        AssistantsError::Transport { .. } => {
            (String::from("APIConnectionError"), None, error.to_string())
        }
        AssistantsError::MissingApiKey => {
            (String::from("OpenAIError"), None, String::from(MISSING_KEY))
        }
        AssistantsError::Malformed { .. }
        | AssistantsError::Truncated { .. }
        | AssistantsError::Shape { .. } => (
            String::from("APIResponseValidationError"),
            None,
            error.to_string(),
        ),
        AssistantsError::BaseUrl { .. }
        | AssistantsError::Header { .. }
        | AssistantsError::Encode { .. }
        | AssistantsError::EmptyAssistantId
        | AssistantsError::DotSegment { .. } => {
            (String::from("ValueError"), None, error.to_string())
        }
    }
}

fn definitions(error: &DefinitionsError) -> (String, Option<u16>, String) {
    match error {
        DefinitionsError::Assistants(assistants_error) => assistants(assistants_error),
        DefinitionsError::Endpoint(endpoint) => (
            endpoint.class().to_owned(),
            endpoint.status(),
            endpoint.to_string(),
        ),
        DefinitionsError::Provider { class, message } => (class.clone(), None, message.clone()),
        DefinitionsError::BaseUrl { .. }
        | DefinitionsError::MissingCredentials
        | DefinitionsError::MissingEndpoint
        | DefinitionsError::ProviderReply
        | DefinitionsError::Timeout { .. } => (String::from("ValueError"), None, error.to_string()),
    }
}

// the pinned openai SDK's status-to-class table inside _make_status_error
fn sdk_class(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        _ if status >= 500 => "InternalServerError",
        _ => "APIStatusError",
    }
}

fn wrapped(
    class: &str,
    status: Option<u16>,
    message: &str,
    raised: &Raised,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> AssistantCreateFailure {
    let body = match raised {
        Raised::Assistants(AssistantsError::Status { body, .. })
        | Raised::Definitions(DefinitionsError::Assistants(AssistantsError::Status {
            body, ..
        })) => flattened(body),
        Raised::Local(_)
        | Raised::Assistants(_)
        | Raised::Definitions(DefinitionsError::Assistants(_))
        | Raised::Definitions(DefinitionsError::Endpoint(_))
        | Raised::Definitions(DefinitionsError::BaseUrl { .. })
        | Raised::Definitions(DefinitionsError::MissingCredentials)
        | Raised::Definitions(DefinitionsError::MissingEndpoint)
        | Raised::Definitions(DefinitionsError::ProviderReply)
        | Raised::Definitions(DefinitionsError::Provider { .. })
        | Raised::Definitions(DefinitionsError::Timeout { .. }) => None,
    };
    let error_str = match status {
        Some(code) => format!("Error code: {code} - {message}"),
        None => message.to_owned(),
    };
    let projected = dispatch(
        &DispatchInput::Raw(Box::new(RawException {
            model: model.to_owned(),
            custom_llm_provider: provider.to_owned(),
            error_str,
            status_code: status,
            status_code_is_synthesized: false,
            message: Some(message.to_owned()),
            body,
            exception_type: class.to_owned(),
        })),
        redactor,
    );
    let metadata = projected.metadata();
    AssistantCreateFailure {
        class: class_name(&projected).to_owned(),
        status: Some(metadata.status_code),
        kind: metadata.error_type,
        param: metadata.param,
        code: metadata
            .openai_code
            .map_or(WireField::Null, WireField::Text),
        message: message.to_owned(),
    }
}

// openai.APIError reads type/param/code from body["error"] when it is a mapping, else from the body itself
fn flattened(body: &[u8]) -> Option<RawExceptionBody> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    let flat = match value {
        Value::Object(mut object) => match object.remove("error") {
            Some(inner @ Value::Object(_)) => inner,
            Some(_) | None => Value::Object(object),
        },
        other => other,
    };
    serde_json::from_value(flat).ok()
}

fn class_name(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "AuthenticationError",
        ErrorClass::NotFound(_) => "NotFoundError",
        ErrorClass::PermissionDenied(_) => "PermissionDeniedError",
        ErrorClass::BadRequest(_) => "BadRequestError",
        ErrorClass::UnprocessableEntity(_) => "UnprocessableEntityError",
        ErrorClass::ContextWindowExceeded(_) => "ContextWindowExceededError",
        ErrorClass::ContentPolicyViolation(_) => "ContentPolicyViolationError",
        ErrorClass::ServiceUnavailable(_) => "ServiceUnavailableError",
        ErrorClass::BadGateway(_) => "BadGatewayError",
        ErrorClass::InternalServer(_) => "InternalServerError",
        ErrorClass::Timeout(_) => "Timeout",
        ErrorClass::RateLimit(_) => "RateLimitError",
        ErrorClass::Api(_) => "APIError",
        ErrorClass::ApiConnection(_) => "APIConnectionError",
        ErrorClass::ApiResponseValidation(_) => "APIResponseValidationError",
        ErrorClass::BudgetExceeded(_) => "BudgetExceededError",
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError",
        ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => "APIError",
    }
}
