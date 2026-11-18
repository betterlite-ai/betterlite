use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::chatgpt_responses::{
    ChatGptResponsesError, ResponsesConfig, transform_reply,
};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::chatgpt::chat::Authenticator;
use betterlite_openai::chatgpt::login::error::LoginError;
use betterlite_openai::response_events::{RawResponse, ReplyBody};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod tests;

const INIT: &str = r#"{"model":"gpt-5","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const LOG_NOW: &str = "2026-01-01T00:00:05+00:00";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    Request,
    Response,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseInput {
    step: Step,
    environment: BTreeMap<String, String>,
    model: String,
    input: String,
    options: Object,
    body: String,
    status: u16,
    headers: IndexMap<String, String>,
    now: i64,
    api_base: String,
    access_token: String,
    account_id: Option<String>,
}

struct ScriptedAuth {
    base: String,
    token: String,
    account: Option<String>,
}

impl Authenticator for ScriptedAuth {
    fn api_base(&self) -> &str {
        &self.base
    }

    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, LoginError>> + Send {
        ready(Ok(SecretString::from(self.token.clone())))
    }

    fn account_id(&mut self) -> Option<String> {
        self.account.clone()
    }
}

struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
}

impl Fixture {
    fn new() -> Result<Self, Box<HelperError>> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
            .map_err(|error| helper_error("ValueError", error.to_string()))?;
        let init: LoggingInit = serde_json::from_str(INIT)
            .map_err(|error| helper_error("ValueError", error.to_string()))?;
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
        Ok(Self {
            redactor,
            sink: Sink,
            state,
        })
    }
}

fn pre_post_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Sink,
) -> Result<PrePostEnv<'env>, Box<HelperError>> {
    Ok(PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339(LOG_NOW)
            .map_err(|error| helper_error("ValueError", error.to_string()))?,
    })
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = CaseInput;

    fn run(input: &Self::Input) -> HelperResult {
        match input.step {
            Step::Request => request_value(input),
            Step::Response => response_value(input),
        }
    }
}

fn request_value(input: &CaseInput) -> Result<Value, Box<HelperError>> {
    let settings = ConfigLoader {
        lookup: input.environment.clone(),
    }
    .load("")
    .map_err(|error| helper_error("ConfigError", error.to_string()))?
    .sections
    .chatgpt_headers;
    let config = ResponsesConfig {
        authenticator: ScriptedAuth {
            base: input.api_base.clone(),
            token: input.access_token.clone(),
            account: input.account_id.clone(),
        },
        header_settings: settings,
    };
    let request = config
        .transform_request(
            &input.model,
            &ResponsesInput::Text(input.input.clone()),
            &input.options,
            None,
        )
        .map_err(|error| responses_error(&error))?;
    serde_json::to_value(request).map_err(|error| helper_error("ValueError", error.to_string()))
}

fn response_value(input: &CaseInput) -> HelperResult {
    let Fixture {
        redactor,
        sink,
        mut state,
    } = Fixture::new()?;
    let env = pre_post_env(&redactor, &sink)?;
    let reply = transform_reply(
        &RawResponse {
            status: input.status,
            headers: &input.headers,
            text: &input.body,
        },
        input.now,
        &mut state,
        &env,
        &ApiLimits::default(),
    )
    .map_err(|error| responses_error(&error))?;
    let ReplyBody::Typed(response) = &reply.response else {
        return Err(helper_error(
            "ValueError",
            String::from("the completed event did not validate"),
        ));
    };
    Ok(json!({
        "value": response_view(response)?,
        "headers": reply.headers,
        "additional_headers": reply.additional_headers,
        "post_call": post_call(&state),
    }))
}

fn response_view(response: &ResponsesApiResponse) -> Result<Value, Box<HelperError>> {
    let value = serde_json::to_value(response)
        .map_err(|error| helper_error("ValueError", error.to_string()))?;
    Ok(json!({
        "id": value.get("id"),
        "created_at": value.get("created_at"),
        "model": value.get("model"),
        "object": value.get("object"),
        "status": value.get("status"),
        "output_text": response.output_text(),
        "output": output_view(value.get("output")),
    }))
}

fn output_view(output: Option<&Value>) -> Vec<Value> {
    output
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |items| {
            items
                .iter()
                .map(|item| json!({"type": item.get("type"), "id": item.get("id")}))
                .collect()
        })
}

fn post_call(state: &LoggingState) -> Value {
    let details = &state.model_call_details;
    json!({
        "original_response": details.get("original_response"),
        "additional_args": details.get("additional_args"),
        "log_event_type": details.get("log_event_type"),
    })
}

fn responses_error(error: &ChatGptResponsesError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("OpenAIError"),
        status: error.http_status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
