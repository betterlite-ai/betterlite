use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_core::api::anthropic_beta_filter::native_injection::NativeRequest;
use betterlite_core::api::messages_native_execute::dispatch::{
    MessagesBaseLlmException, MessagesDispatchOutcome, MessagesDispatchRequest,
};
use betterlite_core::api::messages_native_execute::entry::MessagesEntryResult;
use betterlite_core::api::messages_native_execute::error::MessagesExecuteError;
use betterlite_core::api::messages_native_prepare::environment::validate_environment;
use betterlite_core::api::messages_native_prepare::url::messages_url;
use betterlite_core::api::messages_options::requested_optional_params;
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use url::Url;

use super::scripted::API_BASE;
use super::{Failure, Outcome};
use crate::sdk_http::error::SdkHttpError;
use crate::sdk_http::vector_query_embedding::outbound_client;

const DEADLINE: Duration = Duration::from_secs(120);
const DEFAULT_PROVIDER: &str = "anthropic";

pub(super) async fn execute_dispatch(
    dispatch: &MessagesDispatchRequest,
    base: &str,
    env: &BTreeMap<String, String>,
) -> Result<MessagesDispatchOutcome, SdkHttpError> {
    let params = base_params(dispatch);
    let optional = requested_optional_params(&params, None).map_err(|error| {
        SdkHttpError::Invalid(format!("the optional parameters failed: {error}"))
    })?;
    let request = outbound_request(dispatch, &optional, base)?;
    let client = outbound_client(env)?;
    let response = client
        .send_buffered(request)
        .await
        .map_err(|error| SdkHttpError::Transport(Box::new(error)))?;
    if (200..300).contains(&response.status) {
        let value: Json =
            serde_json::from_slice(&response.body).map_err(SdkHttpError::ErrorBody)?;
        return Ok(MessagesDispatchOutcome::Returned(
            MessagesEntryResult::Response(value),
        ));
    }
    let message = String::from_utf8(response.body).map_err(SdkHttpError::ErrorBodyUtf8)?;
    Ok(MessagesDispatchOutcome::BaseLlmException(
        MessagesBaseLlmException {
            status_code: response.status,
            message,
        },
    ))
}

pub(super) fn dispatch_record(dispatch: &MessagesDispatchRequest) -> IndexMap<String, Json> {
    let mut record: IndexMap<String, Json> = IndexMap::new();
    record.insert(
        "api_base".to_owned(),
        dispatch
            .api_base
            .as_deref()
            .map_or(Json::Null, |_found| Json::Text(API_BASE.to_owned())),
    );
    record.insert("client".to_owned(), Json::Null);
    record.insert(
        "_litellm_messages_presanitized".to_owned(),
        Json::Bool(dispatch.presanitized),
    );
    for (key, value) in &dispatch.kwargs {
        record.insert(key.clone(), value.clone());
    }
    record
}

pub(super) fn record_failure(
    outcome: &mut Outcome,
    error: MessagesExecuteError,
) -> Result<(), SdkHttpError> {
    match error {
        MessagesExecuteError::Raised(raised) => {
            outcome.failure = Some(Failure {
                class: raised.class,
                status: raised.status.unwrap_or(500),
                error_type: raised.kind.map_or(Json::Null, Json::Text),
                param: raised.param.map_or(Json::Null, Json::Text),
                code: raised.code.map_or(Json::Null, Json::Text),
            });
        }
        MessagesExecuteError::Classified(class) => outcome.failure = Some(class_failure(&class)),
        other => return Err(SdkHttpError::Invalid(other.to_string())),
    }
    Ok(())
}

fn class_failure(class: &ErrorClass) -> Failure {
    let metadata = class.metadata();
    Failure {
        class: class_name(class).to_owned(),
        status: metadata.status_code,
        error_type: wire(metadata.error_type),
        param: wire(metadata.param),
        code: metadata.openai_code.map_or(Json::Null, Json::Text),
    }
}

fn wire(field: WireField) -> Json {
    match field {
        WireField::Missing(_) | WireField::Null => Json::Null,
        WireField::Text(text) => Json::Text(text),
    }
}

fn class_name(class: &ErrorClass) -> &'static str {
    match class {
        ErrorClass::Authentication(_) => "AuthenticationError",
        ErrorClass::NotFound(_) => "NotFoundError",
        ErrorClass::PermissionDenied(_) => "PermissionDeniedError",
        ErrorClass::BadRequest(_) => "BadRequestError",
        ErrorClass::UnprocessableEntity(_) => "UnprocessableEntityError",
        ErrorClass::Timeout(_) => "Timeout",
        ErrorClass::RateLimit(_) => "RateLimitError",
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
        ErrorClass::UnsupportedParams(_) => "UnsupportedParamsError",
        ErrorClass::GuardrailRaised(_) => "GuardrailRaisedError",
        ErrorClass::MidStreamFallback(_) => "MidStreamFallbackError",
        ErrorClass::Unclassified(_) => "APIError",
    }
}

fn base_params(dispatch: &MessagesDispatchRequest) -> IndexMap<String, Json> {
    let mut params: IndexMap<String, Json> = IndexMap::new();
    params.insert(
        "max_tokens".to_owned(),
        Json::Number(serde_json::Number::from(dispatch.max_tokens)),
    );
    insert_optional(&mut params, "metadata", dispatch.metadata.as_ref());
    insert_optional(
        &mut params,
        "stop_sequences",
        dispatch.stop_sequences.as_ref(),
    );
    insert_optional(&mut params, "system", dispatch.system.as_ref());
    insert_optional(&mut params, "temperature", dispatch.temperature.as_ref());
    insert_optional(&mut params, "thinking", dispatch.thinking.as_ref());
    insert_optional(&mut params, "tool_choice", dispatch.tool_choice.as_ref());
    insert_optional(&mut params, "top_k", dispatch.top_k.as_ref());
    insert_optional(&mut params, "top_p", dispatch.top_p.as_ref());
    if let Some(tools) = &dispatch.tools {
        params.insert("tools".to_owned(), Json::List(tools.clone()));
    }
    if let Some(stream) = dispatch.stream {
        params.insert("stream".to_owned(), Json::Bool(stream));
    }
    for (key, value) in &dispatch.kwargs {
        params.insert(key.clone(), value.clone());
    }
    params
}

fn insert_optional(params: &mut IndexMap<String, Json>, name: &str, value: Option<&Json>) {
    if let Some(found) = value {
        params.insert(name.to_owned(), found.clone());
    }
}

fn provider_body(
    dispatch: &MessagesDispatchRequest,
    optional: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut body = optional.clone();
    body.insert("model".to_owned(), Json::Text(dispatch.model.clone()));
    body.insert("messages".to_owned(), Json::List(dispatch.messages.clone()));
    body.insert(
        "max_tokens".to_owned(),
        Json::Number(serde_json::Number::from(dispatch.max_tokens)),
    );
    body
}

fn outbound_request(
    dispatch: &MessagesDispatchRequest,
    optional: &IndexMap<String, Json>,
    base: &str,
) -> Result<OutboundRequest, SdkHttpError> {
    let settings = AnthropicSettings::default();
    let prompt_messages: Vec<PromptJson> = transcode(&Json::List(dispatch.messages.clone()))?;
    let prompt_params: IndexMap<String, PromptJson> = transcode(&Json::Object(optional.clone()))?;
    let provider = dispatch
        .custom_llm_provider
        .as_deref()
        .unwrap_or(DEFAULT_PROVIDER);
    let native = NativeRequest {
        provider,
        optional_params: &prompt_params,
        messages: &prompt_messages,
    };
    let mut headers: IndexMap<String, String> = IndexMap::new();
    validate_environment(
        &mut headers,
        dispatch.api_key.clone(),
        Some(base),
        &native,
        &settings,
    )
    .map_err(|error| SdkHttpError::Invalid(format!("the Anthropic environment failed: {error}")))?;
    let endpoint = Url::parse(&messages_url(Some(base), &settings))
        .map_err(|source| SdkHttpError::Url(Box::new(source)))?;
    let encoded =
        serde_json::to_vec(&provider_body(dispatch, optional)).map_err(SdkHttpError::Serialize)?;
    Ok(OutboundRequest {
        method: Method::POST,
        url: endpoint,
        query: Vec::new(),
        headers: header_map(&headers)?,
        body: RequestBody::Json(encoded),
        deadline: DEADLINE,
    })
}

fn transcode<T>(value: &Json) -> Result<T, SdkHttpError>
where
    T: DeserializeOwned,
{
    let encoded = serde_json::to_vec(value).map_err(SdkHttpError::Serialize)?;
    serde_json::from_slice(&encoded).map_err(|error| {
        SdkHttpError::Invalid(format!("the native request is not convertible: {error}"))
    })
}

fn header_map(headers: &IndexMap<String, String>) -> Result<HeaderMap, SdkHttpError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header = HeaderName::from_bytes(name.as_bytes()).map_err(|_source| {
            SdkHttpError::Invalid(format!("the request header {name} is not a name"))
        })?;
        let text = HeaderValue::from_bytes(value.as_bytes()).map_err(|_source| {
            SdkHttpError::Invalid(format!("the request header {name} is not text"))
        })?;
        map.insert(header, text);
    }
    Ok(map)
}
