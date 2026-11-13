use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_openai::error::OpenAiError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, decode_response_event,
};
use betterlite_openai::response_events::{
    RawResponse, ReplyBody, ResponsesReply, transform_response,
};
use betterlite_openai::xai_responses::error::XaiResponsesError;
use betterlite_openai::xai_responses::{
    OauthAccess, get_complete_url, map_openai_params, restate_event_usage, restate_reply_usage,
    validate_environment,
};
use betterlite_openai::xai_tokens::error::XaiOauthError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

const INIT: &str = r#"{"model":"grok-4","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:05:00+00:00";
const DEFAULT_OAUTH_BASE: &str = "https://api.x.ai/v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    env: BTreeMap<String, String>,
    xai_key: Option<String>,
    generic_key: Option<String>,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Tools {
        params: Object,
    },
    Url {
        api_base: Option<String>,
        litellm_api_base: Option<String>,
        api_key: Option<String>,
        #[serde(default)]
        use_xai_oauth: bool,
    },
    Auth {
        api_key: Option<String>,
        #[serde(default)]
        use_xai_oauth: bool,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        oauth: Option<String>,
    },
    Reply {
        status: u16,
        body: String,
        now: i64,
    },
    Event {
        event: Value,
    },
    Capability,
}

struct OauthFixture {
    token: Option<String>,
}

impl OauthAccess for OauthFixture {
    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send {
        let outcome = match self.token.take() {
            Some(token) => Ok(SecretString::from(token)),
            None => Err(XaiOauthError::LoginRequired),
        };
        std::future::ready(outcome)
    }
}

struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

fn failure(message: String, status: Option<u16>) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("FixtureError"),
        status,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn identity_settings(
    xai_key: Option<&str>,
    generic_key: Option<&str>,
    env: &BTreeMap<String, String>,
) -> Settings {
    let read = |name: &str| env.get(name).filter(|value| !value.is_empty()).cloned();
    Settings {
        xai_key: xai_key.map(SecretString::from),
        generic_api_key: generic_key.map(SecretString::from),
        environment_key: read("XAI_API_KEY").map(SecretString::from),
        api_base: read("XAI_API_BASE"),
    }
}

fn oauth_base(env: &BTreeMap<String, String>) -> String {
    let read = |name: &str| env.get(name).filter(|value| !value.is_empty()).cloned();
    read("XAI_OAUTH_API_BASE")
        .or_else(|| read("XAI_API_BASE"))
        .unwrap_or_else(|| String::from(DEFAULT_OAUTH_BASE))
}

fn usage_json(usage: Option<&ResponseApiUsage>) -> Value {
    let Some(usage) = usage else {
        return json!({"cost": Value::Null, "ticks": Value::Null});
    };
    let cost = usage.cost.map(|cost| cost.normalize().to_string());
    let ticks = usage
        .extra
        .get("cost_in_usd_ticks")
        .and_then(|ticks| serde_json::to_value(ticks).ok())
        .unwrap_or(Value::Null);
    json!({"cost": cost, "ticks": ticks})
}

fn known_usage(record: &KnownResponseEvent) -> Option<&ResponseApiUsage> {
    if let KnownResponseEvent::ResponseCompleted(fields) = record {
        return fields.response.usage.as_ref();
    }
    if let KnownResponseEvent::ResponseIncomplete(fields) = record {
        return fields.response.usage.as_ref();
    }
    if let KnownResponseEvent::ResponseFailed(fields) = record {
        return fields.response.usage.as_ref();
    }
    None
}

fn event_value(event: &DecodedStreamResponse) -> Value {
    let usage = match event {
        DecodedStreamResponse::Known { record, .. } => known_usage(record),
        DecodedStreamResponse::Generic(_) | DecodedStreamResponse::Lax { .. } => None,
    };
    let mut value = usage_json(usage);
    if let Value::Object(fields) = &mut value {
        fields.insert(
            String::from("class"),
            Value::String(event.class().class_name().to_owned()),
        );
    }
    value
}

fn reply_usage(reply: &ResponsesReply) -> Value {
    match &reply.response {
        ReplyBody::Typed(response) => usage_json(response.usage.as_ref()),
        ReplyBody::Lax(_fields) => usage_json(None),
    }
}

fn error_status(error: &OpenAiError) -> Option<u16> {
    if let OpenAiError::Provider(provider) = error {
        Some(provider.status_code)
    } else {
        None
    }
}

fn reply_step(status: u16, body: &str, now: i64) -> crate::helpers::HelperResult {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| failure(error.to_string(), None))?;
    let init = serde_json::from_str::<LoggingInit>(INIT)
        .map_err(|error| failure(error.to_string(), None))?;
    let mut state = LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("trace"),
        Duration::default,
    )
    .map_err(|error| failure(error.to_string(), None))?;
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor: &redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339(NOW).map_err(|error| failure(error.to_string(), None))?,
    };
    let headers: IndexMap<String, String> = IndexMap::new();
    let reply = RawResponse {
        status,
        headers: &headers,
        text: body,
    };
    let mut transformed = transform_response(&reply, now, &mut state, &env)
        .map_err(|error| failure(error.to_string(), error_status(&error)))?;
    restate_reply_usage(&mut transformed);
    Ok(reply_usage(&transformed))
}

fn auth_step(
    settings: &Settings,
    api_key: Option<&str>,
    use_xai_oauth: bool,
    headers: &BTreeMap<String, String>,
    oauth: Option<&str>,
) -> crate::helpers::HelperResult {
    let key = api_key.map(SecretString::from);
    let mut fixture = OauthFixture {
        token: oauth.map(str::to_owned),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| failure(error.to_string(), None))?;
    let validated: Result<Headers, XaiResponsesError> = runtime.block_on(validate_environment(
        headers.clone(),
        key.as_ref(),
        "grok-4",
        use_xai_oauth,
        settings,
        &mut fixture,
    ));
    match validated {
        Ok(headers) => {
            serde_json::to_value(headers).map_err(|error| failure(error.to_string(), None))
        }
        Err(error) => Err(failure(error.to_string(), error.status_code())),
    }
}

fn run_step(
    step: &Step,
    settings: &Settings,
    env: &BTreeMap<String, String>,
) -> crate::helpers::HelperResult {
    match step {
        Step::Tools { params } => {
            let mut params = params.clone();
            map_openai_params(&mut params);
            Ok(Value::Object(params))
        }
        Step::Url {
            api_base,
            litellm_api_base,
            api_key,
            use_xai_oauth,
        } => {
            let key = api_key.as_deref().map(SecretString::from);
            Ok(Value::String(get_complete_url(
                api_base.as_deref(),
                litellm_api_base.as_deref(),
                key.as_ref(),
                *use_xai_oauth,
                settings,
                &oauth_base(env),
            )))
        }
        Step::Auth {
            api_key,
            use_xai_oauth,
            headers,
            oauth,
        } => auth_step(
            settings,
            api_key.as_deref(),
            *use_xai_oauth,
            headers,
            oauth.as_deref(),
        ),
        Step::Reply { status, body, now } => reply_step(*status, body, *now),
        Step::Event { event } => {
            let bytes =
                serde_json::to_vec(event).map_err(|error| failure(error.to_string(), None))?;
            let mut decoded =
                decode_response_event(&bytes).map_err(|error| failure(error.to_string(), None))?;
            restate_event_usage(&mut decoded);
            Ok(event_value(&decoded))
        }
        Step::Capability => Ok(json!({
            "native_websocket": betterlite_openai::xai_responses::supports_native_websocket(),
        })),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let settings = identity_settings(
            input.xai_key.as_deref(),
            input.generic_key.as_deref(),
            &input.env,
        );
        // LiteLLM raises per call, so the case keeps every step's answer beside a raised status
        let answers = input
            .steps
            .iter()
            .map(|one| match run_step(one, &settings, &input.env) {
                Ok(value) => value,
                Err(error) => json!({"raised": error.status}),
            })
            .collect::<Vec<Value>>();
        Ok(Value::Array(answers))
    }
}

mod tests;
