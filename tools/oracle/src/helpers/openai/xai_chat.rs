use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::xai_identity::Settings as IdentitySettings;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat_choices::wire::StreamReply;
use betterlite_openai::chat_content::{ChatContent, RefuseDownloads};
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::xai_chat::error::XaiChatError;
use betterlite_openai::xai_chat::params::{
    ReasoningLookup, map_openai_params, supported_openai_params,
};
use betterlite_openai::xai_chat::request::transform_request;
use betterlite_openai::xai_chat::response::{ReplyInput, transform_response};
use betterlite_openai::xai_chat::stream::{XaiStreamError, parse_chunk};
use betterlite_openai::xai_chat::{
    Environment, OauthTokens, PROVIDER, UrlInput, complete_url, provider_info, validate_environment,
};
use betterlite_openai::xai_tokens::error::XaiOauthError;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use uuid::Uuid;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    settings: Settings,
    steps: Vec<Step>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    api_base: Option<String>,
    oauth_api_base: Option<String>,
    xai_key: Option<SecretString>,
    environment_key: Option<SecretString>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    Supported {
        model: String,
        supports_reasoning: Option<bool>,
    },
    Map {
        model: String,
        supports_reasoning: bool,
        params: Vec<(String, Json)>,
        optional_params: Object,
    },
    Request {
        model: String,
        messages: Vec<ChatMessage>,
        optional_params: Object,
    },
    Environment {
        model: String,
        headers: Headers,
        api_key: Option<SecretString>,
        use_xai_oauth: bool,
    },
    Url {
        api_base: Option<String>,
        api_key: Option<SecretString>,
        use_xai_oauth: bool,
    },
    Reply {
        body: String,
    },
    Chunk {
        chunk: Value,
    },
}

struct Oauth {
    api_base: String,
    token: Option<&'static str>,
}

impl OauthTokens for Oauth {
    fn api_base(&self) -> String {
        self.api_base.clone()
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send {
        let token = self.token;
        async move {
            match token {
                Some(value) => Ok(SecretString::from(String::from(value))),
                None => Err(XaiOauthError::LoginRequired),
            }
        }
    }
}

struct CapabilityLookup {
    outcome: Option<bool>,
}

struct LookupFailure;

impl fmt::Display for LookupFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the capability lookup failed")
    }
}

impl ReasoningLookup<LookupFailure> for CapabilityLookup {
    fn supports_reasoning(
        &mut self,
        _model: &str,
    ) -> impl Future<Output = Result<bool, LookupFailure>> + Send {
        let outcome = self.outcome;
        async move { outcome.ok_or(LookupFailure) }
    }
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Warnings {
    fn count(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn failure(error: XaiChatError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("XaiChatError"),
        status: Some(error.status_code()),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn stream_failure(error: XaiStreamError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("XaiStreamError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn identity(settings: &Settings) -> IdentitySettings {
    IdentitySettings {
        xai_key: settings.xai_key.clone(),
        generic_api_key: None,
        environment_key: settings.environment_key.clone(),
        api_base: settings.api_base.clone(),
    }
}

fn oauth(settings: &Settings, token: Option<&'static str>) -> Oauth {
    Oauth {
        api_base: settings
            .oauth_api_base
            .clone()
            .unwrap_or_else(|| String::from("https://api.x.ai/v1")),
        token,
    }
}

fn zero_clock() -> i64 {
    0
}

fn decimal_text(value: Option<&Value>) -> Option<String> {
    let Value::Number(number) = value? else {
        return None;
    };
    let decimal = parse_decimal_lexeme(&number.to_string()).ok()?;
    Some(decimal.normalize().to_string())
}

fn stream_usage(reply: &StreamReply) -> Value {
    let Some(Value::Object(fields)) = reply.usage.as_ref() else {
        return Value::Null;
    };
    json!({
        "prompt_tokens": fields.get("prompt_tokens"),
        "completion_tokens": fields.get("completion_tokens"),
        "total_tokens": fields.get("total_tokens"),
        "cost": decimal_text(fields.get("cost")),
    })
}

struct Run<'request> {
    settings: &'request Settings,
    runtime: &'request tokio::runtime::Runtime,
}

impl Run<'_> {
    fn supported(&self, model: &str, supports_reasoning: Option<bool>) -> Value {
        let mut lookup = CapabilityLookup {
            outcome: supports_reasoning,
        };
        let params = self
            .runtime
            .block_on(supported_openai_params(model, &mut lookup));
        json!({ "supported": params })
    }

    fn mapped(
        &self,
        model: &str,
        supports_reasoning: bool,
        params: &[(String, Json)],
        optional_params: &Object,
    ) -> HelperResult {
        let mut optional = optional_params.clone();
        let requested = Object::from_iter(params.iter().cloned());
        map_openai_params(model, supports_reasoning, &requested, &mut optional).map_err(failure)?;
        Ok(json!({ "mapped": optional }))
    }

    fn requested(
        &self,
        model: &str,
        messages: &[ChatMessage],
        optional_params: &Object,
    ) -> HelperResult {
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        let environment = BTreeMap::<String, String>::new();
        let body = with_default(subscriber, || {
            transform_request(ChatContent {
                model,
                messages,
                optional_params,
                provider: Some(PROVIDER),
                api_base: None,
                global_api_base: None,
                openai_system_messages_first: false,
                environment: &environment,
                downloads: &RefuseDownloads,
            })
        })
        .map_err(failure)?;
        Ok(json!({ "body": body, "warnings": warnings.count() }))
    }

    fn environment(
        &self,
        model: &str,
        headers: &Headers,
        api_key: Option<&SecretString>,
        use_xai_oauth: bool,
    ) -> HelperResult {
        let settings = identity(self.settings);
        let mut authenticator = oauth(self.settings, None);
        let resolved = self
            .runtime
            .block_on(validate_environment(Environment {
                headers: headers.clone(),
                api_key,
                settings: &settings,
                model,
                use_xai_oauth,
                oauth: &mut authenticator,
            }))
            .map_err(failure)?;
        Ok(json!({ "headers": resolved }))
    }

    fn url(
        &self,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
        use_xai_oauth: bool,
    ) -> Value {
        let settings = identity(self.settings);
        let authenticator = oauth(self.settings, None);
        let resolved = provider_info(api_base, api_key, &settings);
        let url = complete_url(UrlInput {
            api_base,
            api_key,
            settings: &settings,
            use_xai_oauth,
            oauth: &authenticator,
        });
        json!({
            "url": url,
            "api_base": resolved.api_base,
            "api_key": resolved.api_key.map(|key| key.expose_secret().to_owned()),
        })
    }

    fn reply(&self, body: &str) -> HelperResult {
        let converted = transform_response(&ReplyInput {
            body: body.as_bytes(),
        })
        .map_err(failure)?;
        let choices: Vec<Value> = converted
            .response
            .choices
            .iter()
            .map(|choice| {
                json!({
                    "finish_reason": choice.finish_reason,
                    "tool_calls": choice.message.tool_calls.as_ref().map_or(0, Vec::len),
                })
            })
            .collect();
        let usage = &converted.usage;
        Ok(json!({
            "choices": choices,
            "usage": {
                "prompt_tokens": usage.tokens.prompt_tokens,
                "completion_tokens": usage.tokens.completion_tokens,
                "total_tokens": usage.tokens.total_tokens,
                "cost": usage.cost.map(|cost| cost.normalize().to_string()),
                "web_search_requests": usage.tokens.prompt_details.and_then(|details| details.web_search_requests),
            },
        }))
    }

    fn chunk(&self, raw: &Value) -> HelperResult {
        let mut chunk = raw.clone();
        let reply = parse_chunk(&mut chunk, Uuid::nil, zero_clock).map_err(stream_failure)?;
        let choices: Vec<Value> = reply
            .choices
            .iter()
            .map(|choice| {
                json!({
                    "index": choice.index,
                    "finish_reason": choice.finish_reason,
                })
            })
            .collect();
        Ok(json!({ "choices": choices, "usage": stream_usage(&reply) }))
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| invalid(error.to_string()))?;
        let run = Run {
            settings: &input.settings,
            runtime: &runtime,
        };
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            results.push(match step {
                Step::Supported {
                    model,
                    supports_reasoning,
                } => run.supported(model, *supports_reasoning),
                Step::Map {
                    model,
                    supports_reasoning,
                    params,
                    optional_params,
                } => run.mapped(model, *supports_reasoning, params, optional_params)?,
                Step::Request {
                    model,
                    messages,
                    optional_params,
                } => run.requested(model, messages, optional_params)?,
                Step::Environment {
                    model,
                    headers,
                    api_key,
                    use_xai_oauth,
                } => run.environment(model, headers, api_key.as_ref(), *use_xai_oauth)?,
                Step::Url {
                    api_base,
                    api_key,
                    use_xai_oauth,
                } => run.url(api_base.as_deref(), api_key.as_ref(), *use_xai_oauth),
                Step::Reply { body } => run.reply(body)?,
                Step::Chunk { chunk } => run.chunk(chunk)?,
            });
        }
        Ok(Value::Array(results))
    }
}
