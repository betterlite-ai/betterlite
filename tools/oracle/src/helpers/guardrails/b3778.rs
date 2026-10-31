use std::collections::BTreeMap;
use std::fmt;
use std::net::TcpListener;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::lasso_classification::Configured;
use betterlite_guardrails::lasso::error::LassoError;
use betterlite_guardrails::lasso::wire::{LassoRequestData, LassoResponse, MessageType};
use betterlite_guardrails::lasso::{ConversationCache, DEFAULT_TIMEOUT, LassoGuardrail};
use reqwest::header::HeaderMap;
use serde_json::json;
use serde_json::value::RawValue;
use ulid::Ulid;

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};
use crate::mock::MockProvider;
use crate::routing_mock_scripts::MockScript;

type Settings = BTreeMap<String, String>;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Construct {
        settings: Settings,
        environment: BTreeMap<String, String>,
    },
    ConversationId {
        settings: Settings,
        data: LassoRequestData,
        cached: Option<String>,
    },
    Headers {
        settings: Settings,
        data: LassoRequestData,
    },
    Payload {
        settings: Settings,
        data: LassoRequestData,
        messages: Vec<Box<RawValue>>,
        message_type: MessageType,
    },
    ProcessResponse {
        settings: Settings,
        response: LassoResponse,
    },
    Call {
        settings: Settings,
        answer: Option<UpstreamAnswer>,
        api_path: Option<String>,
    },
}

#[derive(Default)]
struct Cache(Mutex<BTreeMap<String, String>>);

impl ConversationCache for Cache {
    type Error = fmt::Error;

    fn get_cache(&self, key: &str) -> Result<Option<String>, fmt::Error> {
        let entries = self.0.lock().map_err(|_poisoned| fmt::Error)?;
        Ok(entries.get(key).cloned())
    }

    fn set_cache(&self, key: &str, value: &str, _ttl: Duration) -> Result<(), fmt::Error> {
        let mut entries = self.0.lock().map_err(|_poisoned| fmt::Error)?;
        entries.insert(key.to_owned(), value.to_owned());
        drop(entries);
        Ok(())
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cache = Cache::default();
        match input {
            Input::Construct {
                settings,
                environment,
            } => construct(settings, environment),
            Input::ConversationId {
                settings,
                data,
                cached,
            } => conversation(settings, data, cached.as_deref()),
            Input::Headers { settings, data } => headers(settings, data),
            Input::Payload {
                settings,
                data,
                messages,
                message_type,
            } => {
                let payload = guardrail(settings)?.prepare_payload(
                    messages.clone(),
                    data,
                    *message_type,
                    &cache,
                );
                serde_json::to_value(payload)
                    .map_err(|error| failure("SerializationError", error.to_string()))
            }
            Input::ProcessResponse { settings, response } => {
                let result = guardrail(settings)?.process_response(response.clone());
                result
                    .map(|()| json!({"allowed": true}))
                    .map_err(|error| lasso_failure(&error))
            }
            Input::Call {
                settings,
                answer,
                api_path,
            } => call(settings, answer.as_ref(), api_path.as_deref()),
        }
    }
}

fn construct(settings: &Settings, environment: &BTreeMap<String, String>) -> HelperResult {
    let configured = configure(settings, environment.clone())?;
    let api_base = configured.api_base;
    let user_id = configured.user_id;
    let conversation_id = configured.conversation_id;
    Ok(json!({"api_base": api_base, "user_id": user_id, "conversation_id": conversation_id}))
}

fn headers(settings: &Settings, data: &LassoRequestData) -> HelperResult {
    let call_id = data.litellm_call_id.as_deref();
    let prepared = guardrail(settings)?.prepare_headers(call_id, &Cache::default());
    let header_map = prepared.map_err(|error| lasso_failure(&error))?;
    let pairs = header_map
        .iter()
        .map(|(name, value)| (name.as_str(), value.to_str().unwrap_or_default()));
    Ok(json!(pairs.collect::<BTreeMap<_, _>>()))
}

fn conversation(
    settings: &Settings,
    data: &LassoRequestData,
    cached: Option<&str>,
) -> HelperResult {
    let cache = Cache::default();
    let call_id = data.litellm_call_id.as_deref();
    let key = format!("lasso_conversation_id:{}", call_id.unwrap_or("None"));
    if let Some(value) = cached {
        let seeded = cache.set_cache(&key, value, Duration::ZERO);
        seeded.map_err(|error| failure("CacheError", error.to_string()))?;
    }
    let id = guardrail(settings)?.conversation_id(call_id, &cache);
    let stored = cache
        .get_cache(&key)
        .map_err(|error| failure("CacheError", error.to_string()))?;
    Ok(json!({"stored": stored.as_deref() == Some(id.as_str()), "id": id}))
}

fn call(
    settings: &Settings,
    answer: Option<&UpstreamAnswer>,
    api_path: Option<&str>,
) -> HelperResult {
    let io = |error: std::io::Error| failure("OSError", error.to_string());
    let mock_error = |error: crate::OracleError| failure("MockError", error.to_string());
    // with no answer the listener accepts the connection and never replies, so the call times out
    let silent = TcpListener::bind(("127.0.0.1", 0)).map_err(io)?;
    let start =
        |reply| MockScript::try_from(std::slice::from_ref(reply)).and_then(MockProvider::start);
    let mock = answer.map(start).transpose().map_err(mock_error)?;
    let base = match &mock {
        Some(provider) => provider.base_url.clone(),
        None => format!("http://{}", silent.local_addr().map_err(io)?),
    };
    let mut lasso = guardrail(settings)?;
    lasso.settings.api_base.clone_from(&base);
    lasso.timeout = Duration::from_millis(200);
    let data = LassoRequestData {
        litellm_call_id: None,
        tools: Vec::new(),
    };
    let payload = lasso.prepare_payload(Vec::new(), &data, MessageType::Prompt, &Cache::default());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io)?;
    let url = api_path.map(|path| format!("{base}{path}"));
    let response = runtime.block_on(lasso.call_api(HeaderMap::new(), &payload, url.as_deref()));
    let calls = mock
        .map(MockProvider::finish)
        .transpose()
        .map_err(mock_error)?;
    let path = calls
        .unwrap_or_default()
        .first()
        .map(|recorded| recorded.request.path.clone());
    Ok(json!({"path": path, "response": response.map_err(|error| lasso_failure(&error))?}))
}

fn configure(
    settings: &Settings,
    lookup: BTreeMap<String, String>,
) -> Result<Configured, Box<HelperError>> {
    let mut parameters = settings.clone();
    parameters.insert("guardrail".to_owned(), "lasso".to_owned());
    let entry = json!({"guardrail_name": "lasso", "litellm_params": parameters});
    let loaded = ConfigLoader { lookup }.load(&json!({"guardrails": [entry]}).to_string());
    let config = loaded.map_err(|error| {
        let class = match error {
            ConfigError::MissingEnvironment { .. } => "LassoGuardrailMissingSecrets",
            ConfigError::Yaml { .. }
            | ConfigError::Unsupported { .. }
            | ConfigError::InvalidFloat { .. }
            | ConfigError::InvalidInteger { .. }
            | ConfigError::InvalidScalar { .. } => "ConfigError",
        };
        failure(class, error.to_string())
    })?;
    let guardrails = config.sections.lasso_classification.guardrails;
    guardrails
        .into_iter()
        .next()
        .ok_or_else(|| failure("ConfigError", "no Lasso guardrail".to_owned()))
}

fn guardrail(settings: &Settings) -> Result<LassoGuardrail, Box<HelperError>> {
    let configured = configure(settings, BTreeMap::new())?;
    Ok(LassoGuardrail {
        settings: configured,
        client: reqwest::Client::default(),
        timeout: DEFAULT_TIMEOUT,
        new_id: Ulid::generate,
    })
}

fn lasso_failure(error: &LassoError) -> Box<HelperError> {
    let class = match error {
        LassoError::Blocked { .. } => "HTTPException",
        LassoError::InvalidApiKey => "LassoGuardrailMissingSecrets",
        LassoError::InvalidHeader { .. }
        | LassoError::Timeout { .. }
        | LassoError::RateLimited
        | LassoError::HttpStatus { .. }
        | LassoError::Request { .. }
        | LassoError::ResponseTooLarge { .. }
        | LassoError::ResponseJson { .. } => "LassoGuardrailAPIError",
    };
    failure(class, error.to_string())
}

fn failure(class_name: &str, message: String) -> Box<HelperError> {
    let class = class_name.to_owned();
    Box::new(HelperError {
        class,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
