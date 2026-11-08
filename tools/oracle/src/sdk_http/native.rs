use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::Config as LoadedConfig;
use betterlite_config::types::ProxyTrust;
use betterlite_core::mock::{chat::response as mock_chat, embedding::response as mock_embedding};
use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use betterlite_error::openai::{OpenaiExceptionInput, body::OpenaiErrorBody, map_openai_exception};
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::MockSettings;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use serde::Serialize;
use serde_json::Value;
use tokio::runtime::Builder;
use tokio::time::timeout;
use uuid::Uuid;

use super::capture::{Execution, Stop};
use super::error::SdkHttpError;
use super::result::{ChatCompletion, Choice, Failure, Nullable, SdkResult};
use super::{ChatMessage, OpenaiChat};

const POLL: Duration = Duration::from_millis(10);

#[derive(Serialize)]
struct WireChat<'wire> {
    model: &'wire str,
    messages: &'wire [ChatMessage],
}

pub(crate) fn execute(
    chat: &OpenaiChat,
    api_base: &str,
    env: &BTreeMap<String, String>,
    stop: &Stop<'_>,
) -> Execution {
    let result = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(SdkHttpError::Io)
        .and_then(|runtime| runtime.block_on(send_until_stopped(chat, api_base, env, stop)));
    Execution {
        result,
        log: Vec::new(),
    }
}

// the request future is dropped, and its connection with it, as soon as the stop fires
async fn send_until_stopped(
    chat: &OpenaiChat,
    api_base: &str,
    env: &BTreeMap<String, String>,
    stop: &Stop<'_>,
) -> Result<SdkResult, SdkHttpError> {
    let mut sending = Box::pin(send(chat, api_base, env));
    loop {
        stop.check()?;
        if let Ok(sent) = timeout(POLL, &mut sending).await {
            return sent;
        }
    }
}

#[derive(Serialize)]
struct Config<'config> {
    model_list: [Entry<'config>; 1],
}

#[derive(Serialize)]
struct Entry<'config> {
    model_name: &'config str,
    litellm_params: Params<'config>,
}

#[derive(Serialize)]
struct Params<'config> {
    model: String,
    api_base: &'config str,
    api_key: &'config str,
    timeout: u64,
}

fn load_config(
    chat: &OpenaiChat,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<LoadedConfig, SdkHttpError> {
    let config = Config {
        model_list: [Entry {
            model_name: &chat.model,
            litellm_params: Params {
                model: format!("openai/{}", chat.model),
                api_base,
                api_key: &chat.api_key,
                timeout: chat.timeout_seconds,
            },
        }],
    };
    let loader = ConfigLoader {
        lookup: env.clone(),
    };
    let text = serde_json::to_string(&config).map_err(SdkHttpError::Serialize)?;
    loader
        .load(&text)
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))
}

fn deployments(
    chat: &OpenaiChat,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Vec<ChatDeployment>, SdkHttpError> {
    let parsed = load_config(chat, api_base, env)?;
    let defaults = openai_defaults(&parsed);
    let mut resolved = Vec::new();
    for entry in &parsed.model_list {
        let deployment = ChatDeployment::resolve(entry, &defaults)
            .map_err(|error| SdkHttpError::Deployment(Box::new(error)))?;
        resolved.push(deployment);
    }
    Ok(resolved)
}

// the same defaults the server builds, so proxy-wide base URLs, keys and the organization resolve from the case env
pub(crate) fn openai_defaults(config: &LoadedConfig) -> OpenAiDefaults {
    let settings = &config.sections.resource_sdk_settings;
    let bases = [
        &settings.api_base,
        &settings.openai_base_url,
        &settings.openai_api_base,
    ];
    let keys = [
        &settings.api_key,
        &settings.openai_key,
        &settings.openai_api_key,
    ];
    OpenAiDefaults {
        request_timeout: config.litellm_settings.request_timeout,
        api_bases: bases.into_iter().flatten().cloned().collect(),
        api_keys: keys.into_iter().flatten().cloned().collect(),
        organization: settings
            .organization
            .clone()
            .or_else(|| settings.openai_organization.clone()),
    }
}

async fn send(
    chat: &OpenaiChat,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    let executor = ChatExecutor {
        client: ProxiedClient::outbound(&policy)
            .map_err(|error| SdkHttpError::Client(Box::new(error)))?,
        deployments: deployments(chat, api_base, env)?,
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
        random: Mutex::new(StdRng::seed_from_u64(0)),
        clock: || 0,
        next_id: Uuid::new_v4,
        mock: MockSettings::default(),
        mock_chat,
        mock_embedding,
    };
    let wire = WireChat {
        model: &chat.model,
        messages: &chat.messages,
    };
    let body = serde_json::to_vec(&wire).map_err(SdkHttpError::Serialize)?;
    let request =
        ChatRequest::parse(&body).map_err(|source| SdkHttpError::ChatParse(Box::new(source)))?;
    let deployment = executor
        .deployments
        .first()
        .ok_or_else(|| SdkHttpError::Invalid(format!("no deployment serves {}", chat.model)))?;
    match executor.send(deployment, &request).await {
        Ok(outcome) => Ok(SdkResult::Success {
            value: completion(outcome.response),
        }),
        Err(ChatError::Provider { response }) => project_failure(&chat.model, &response),
        Err(other) => Err(SdkHttpError::Chat(Box::new(other))),
    }
}

fn completion(response: ChatResponse) -> ChatCompletion {
    ChatCompletion {
        id: response.id,
        created: response.created,
        object: response.object,
        model: Some(response.model),
        choices: response
            .choices
            .into_iter()
            .map(|choice| Choice {
                content: choice.message.content,
                finish_reason: choice.finish_reason,
            })
            .collect(),
    }
}

// the OpenAI SDK reads type/param/code from `body["error"]` when it is a mapping, else from the body itself
fn project_failure(model: &str, response: &UpstreamResponse) -> Result<SdkResult, SdkHttpError> {
    let text = String::from_utf8(response.body.clone()).map_err(SdkHttpError::ErrorBodyUtf8)?;
    let flat = match serde_json::from_str::<Value>(&text).ok() {
        Some(Value::Object(mut object)) => match object.remove("error") {
            Some(inner @ Value::Object(_)) => Some(inner),
            Some(_) | None => Some(Value::Object(object)),
        },
        Some(_) | None => None,
    };
    let input = OpenaiExceptionInput {
        model: model.to_owned(),
        custom_llm_provider: "openai".to_owned(),
        error_str: format!("Error code: {} - {text}", response.status),
        status_code: Some(response.status),
        status_code_is_synthesized: false,
        message: None,
        body: flat
            .map(serde_json::from_value::<OpenaiErrorBody>)
            .transpose()
            .map_err(SdkHttpError::ErrorBody)?,
    };
    let class = map_openai_exception(&input);
    let (name, message) = class_name(&class)?;
    let metadata = class.metadata();
    Ok(SdkResult::Failure(Failure {
        class: name.to_owned(),
        status: metadata.status_code,
        kind: field(metadata.error_type),
        param: field(metadata.param),
        code: Some(metadata.openai_code),
        message: message.to_owned(),
    }))
}

fn field(wire: WireField) -> Nullable {
    match wire {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}

// only the classes the pinned SDK raises as openai.APIError subclasses have a projection here
fn class_name(class: &ErrorClass) -> Result<(&'static str, &str), SdkHttpError> {
    match class {
        ErrorClass::Authentication(details) => Ok(("AuthenticationError", &details.message)),
        ErrorClass::NotFound(details) => Ok(("NotFoundError", &details.message)),
        ErrorClass::PermissionDenied(details) => Ok(("PermissionDeniedError", &details.message)),
        ErrorClass::BadRequest(details) => Ok(("BadRequestError", &details.message)),
        ErrorClass::UnprocessableEntity(details) => {
            Ok(("UnprocessableEntityError", &details.message))
        }
        ErrorClass::ContextWindowExceeded(details) => {
            Ok(("ContextWindowExceededError", &details.message))
        }
        ErrorClass::ContentPolicyViolation(details) => {
            Ok(("ContentPolicyViolationError", &details.message))
        }
        ErrorClass::ServiceUnavailable(context) => {
            Ok(("ServiceUnavailableError", &context.message))
        }
        ErrorClass::BadGateway(context) => Ok(("BadGatewayError", &context.message)),
        ErrorClass::InternalServer(context) => Ok(("InternalServerError", &context.message)),
        ErrorClass::Timeout(details) => Ok(("Timeout", &details.context.message)),
        ErrorClass::RateLimit(details) => Ok(("RateLimitError", &details.context.message)),
        ErrorClass::Api(details) => Ok(("APIError", &details.context.message)),
        ErrorClass::ApiConnection(context) => Ok(("APIConnectionError", &context.message)),
        ErrorClass::BudgetExceeded(_)
        | ErrorClass::ApiResponseValidation(_)
        | ErrorClass::JsonSchemaValidation(_)
        | ErrorClass::UnsupportedParams(_)
        | ErrorClass::GuardrailRaised(_)
        | ErrorClass::MidStreamFallback(_)
        | ErrorClass::Unclassified(_) => Err(SdkHttpError::Unprojected(format!("{class:?}"))),
    }
}
