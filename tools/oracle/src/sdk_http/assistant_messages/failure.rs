use betterlite_core::api::assistant_messages::error::{MessageError, MessageFailure, SetupError};
use betterlite_core::api::assistant_messages::options::OptionError;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use serde_json::Value;

use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::result::{Failure, Nullable};

use super::RunOutcome;

// the raw openai SDK exception the sync wrappers let through
pub(super) fn sync(error: &MessageError) -> Result<Failure, SdkHttpError> {
    match error {
        MessageError::Setup(setup) => setup_failure(setup),
        MessageError::OpenAi(_) | MessageError::Azure(_) => provider_failure(error),
        MessageError::UnexpectedReply { .. } => Err(SdkHttpError::Invalid(format!(
            "the provider answered the message call with the wrong SDK object: {error}"
        ))),
    }
}

fn provider_failure(error: &MessageError) -> Result<Failure, SdkHttpError> {
    let Some((status, body)) = error.provider_answer() else {
        return Err(SdkHttpError::Invalid(format!(
            "the message call failed without a provider status: {error}"
        )));
    };
    let (kind, param, code) = body_fields(body);
    Ok(Failure {
        class: status_class(status).to_owned(),
        status,
        kind,
        param,
        code,
        message: format!(
            "Error code: {status} - {}",
            String::from_utf8(body.to_vec()).unwrap_or_default()
        ),
    })
}

// the selector's own exception reaches the caller before any provider client exists
fn setup_failure(setup: &SetupError) -> Result<Failure, SdkHttpError> {
    match setup {
        SetupError::Options(options) if !matches!(options, OptionError::MalformedAttachment) => {
            Ok(Failure {
                class: options.exception_name().to_owned(),
                status: options.status().unwrap_or(400),
                // litellm.UnsupportedParamsError never reaches openai.APIError.__init__, so type stays unset
                kind: None,
                param: Some(None),
                code: Some(None),
                message: options.litellm_message(),
            })
        }
        SetupError::UnsupportedProvider { .. } => Ok(Failure {
            class: "BadRequestError".to_owned(),
            status: 400,
            kind: Some(None),
            param: Some(None),
            code: Some(None),
            message: format!("litellm.BadRequestError: {setup}"),
        }),
        SetupError::Options(_)
        | SetupError::Timeout(_)
        | SetupError::Data { .. }
        | SetupError::Deadline { .. } => Err(SdkHttpError::Invalid(format!(
            "the message setup failed outside a LiteLLM exception: {setup}"
        ))),
    }
}

// exception_type(model="", custom_llm_provider=..., original_exception=...)
pub(super) fn mapped(failure: &MessageFailure) -> Failure {
    let metadata = failure.class.metadata();
    Failure {
        class: class_name(&failure.class).to_owned(),
        status: metadata.status_code,
        kind: nullable(metadata.error_type),
        param: nullable(metadata.param),
        code: Some(metadata.openai_code),
        message: failure_message(&failure.error),
    }
}

fn failure_message(error: &MessageError) -> String {
    match error {
        MessageError::Setup(SetupError::Options(options)) => options.litellm_message(),
        MessageError::Setup(SetupError::UnsupportedProvider { .. }) => {
            format!("litellm.BadRequestError: {error}")
        }
        MessageError::Setup(
            SetupError::Timeout(_) | SetupError::Data { .. } | SetupError::Deadline { .. },
        )
        | MessageError::OpenAi(_)
        | MessageError::Azure(_)
        | MessageError::UnexpectedReply { .. } => error.to_string(),
    }
}

fn nullable(field: WireField) -> Nullable {
    match field {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

// openai._client._make_status_error picks the class by status alone
const fn status_class(status: u16) -> &'static str {
    match status {
        400 => "BadRequestError",
        401 => "AuthenticationError",
        403 => "PermissionDeniedError",
        404 => "NotFoundError",
        409 => "ConflictError",
        422 => "UnprocessableEntityError",
        429 => "RateLimitError",
        500..=u16::MAX => "InternalServerError",
        _ => "APIStatusError",
    }
}

// type(exception).__name__ after LiteLLM's exception_type mapping
const fn class_name(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "AuthenticationError",
        ErrorClass::NotFound(_) => "NotFoundError",
        ErrorClass::PermissionDenied(_) => "PermissionDeniedError",
        ErrorClass::BadRequest(_) => "BadRequestError",
        ErrorClass::UnprocessableEntity(_) => "UnprocessableEntityError",
        ErrorClass::RateLimit(_) => "RateLimitError",
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError",
        ErrorClass::Timeout(_) => "Timeout",
        ErrorClass::BudgetExceeded(_) => "BudgetExceededError",
        ErrorClass::ContextWindowExceeded(_) => "ContextWindowExceededError",
        ErrorClass::ContentPolicyViolation(_) => "ContentPolicyViolationError",
        ErrorClass::ServiceUnavailable(_) => "ServiceUnavailableError",
        ErrorClass::BadGateway(_) => "BadGatewayError",
        ErrorClass::InternalServer(_) => "InternalServerError",
        ErrorClass::Api(_) => "APIError",
        ErrorClass::ApiConnection(_) => "APIConnectionError",
        ErrorClass::ApiResponseValidation(_) => "APIResponseValidationError",
        ErrorClass::JsonSchemaValidation(_) => "JSONSchemaValidationError",
        ErrorClass::GuardrailRaised(_) => "GuardrailRaisedException",
        ErrorClass::MidStreamFallback(_) => "MidStreamFallbackError",
        ErrorClass::Unclassified(_) => "Exception",
    }
}

// _make_status_error hands APIError the object under "error" when the parsed body is a mapping (openai/_client.py#L687)
fn body_fields(body: &[u8]) -> (Nullable, Nullable, Nullable) {
    let Ok(Value::Object(mut fields)) = serde_json::from_slice::<Value>(body) else {
        return (Some(None), Some(None), Some(None));
    };
    let flat = match fields.remove("error") {
        Some(Value::Object(inner)) => inner,
        // a non-mapping body leaves APIError's type/param/code unset
        Some(_) => return (Some(None), Some(None), Some(None)),
        None => fields,
    };
    let kind = flat.get("type").and_then(Value::as_str).map(str::to_owned);
    let param = flat.get("param").and_then(Value::as_str).map(str::to_owned);
    let code = flat.get("code").and_then(Value::as_str).map(str::to_owned);
    (Some(kind), Some(param), Some(code))
}

impl From<Failure> for RunOutcome {
    fn from(failure: Failure) -> Self {
        Self {
            value: None,
            failure: Some(failure),
        }
    }
}
