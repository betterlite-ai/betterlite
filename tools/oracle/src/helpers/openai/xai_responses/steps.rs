use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_logging::logging_pre_post::env::{DebugSettings, GlobalFlags, PrePostEnv};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::response_events::event_decode::decode_response_event;
use betterlite_openai::response_events::{RawResponse, transform_response};
use betterlite_openai::xai_responses::{
    Environment, SUPPORTS_NATIVE_WEBSOCKET, UrlInput, get_complete_url, map_openai_params,
    restate_event_usage, restate_reply_usage, validate_environment,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::values::{error_status, event_value, reply_usage};
use super::{OauthFixture, Silent, Step, oauth_base};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use tokio::runtime::Builder;

const INIT: &str = r#"{"model":"grok-4","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:05:00+00:00";

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

fn reply_step(status: u16, body: &str, now: i64) -> HelperResult {
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
) -> HelperResult {
    let key = api_key.map(SecretString::from);
    let mut fixture = OauthFixture;
    let runtime = Builder::new_current_thread()
        .build()
        .map_err(|error| failure(error.to_string(), None))?;
    let validated = runtime.block_on(validate_environment(Environment {
        headers: headers.clone(),
        api_key: key.as_ref(),
        model: "grok-4",
        use_xai_oauth,
        settings,
        oauth: &mut fixture,
    }));
    match validated {
        Ok(resolved) => {
            serde_json::to_value(resolved).map_err(|error| failure(error.to_string(), None))
        }
        Err(error) => Err(failure(error.to_string(), error.status_code())),
    }
}

fn event_step(event: &Value) -> HelperResult {
    let bytes = serde_json::to_vec(event).map_err(|error| failure(error.to_string(), None))?;
    let mut decoded =
        decode_response_event(&bytes).map_err(|error| failure(error.to_string(), None))?;
    restate_event_usage(&mut decoded);
    Ok(event_value(&decoded))
}

pub(super) fn run_step(
    step: &Step,
    settings: &Settings,
    env: &BTreeMap<String, String>,
) -> HelperResult {
    match step {
        Step::Tools { params } => {
            let mut transformed = params.clone();
            map_openai_params(&mut transformed);
            Ok(Value::Object(transformed))
        }
        Step::Url {
            api_base,
            litellm_api_base,
            api_key,
            use_xai_oauth,
        } => {
            let key = api_key.as_deref().map(SecretString::from);
            let base = oauth_base(env);
            Ok(Value::String(get_complete_url(&UrlInput {
                api_base: api_base.as_deref(),
                litellm_api_base: litellm_api_base.as_deref(),
                api_key: key.as_ref(),
                use_xai_oauth: *use_xai_oauth,
                settings,
                oauth_base: &base,
            })))
        }
        Step::Auth {
            api_key,
            use_xai_oauth,
            headers,
        } => auth_step(settings, api_key.as_deref(), *use_xai_oauth, headers),
        Step::Reply { status, body, now } => reply_step(*status, body, *now),
        Step::Event { event } => event_step(event),
        Step::Capability => Ok(json!({
            "native_websocket": SUPPORTS_NATIVE_WEBSOCKET,
        })),
    }
}
