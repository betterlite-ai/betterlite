use std::collections::BTreeMap;

use betterlite_redact::sentry::pattern::scrub as scrub_text;
use betterlite_redact::sentry::scrub::{EventValue, scrub_event};
use betterlite_redact::sentry::settings::{build_sentry_init_options, send_default_pii_from_env};

#[derive(serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ScrubText {
        text: String,
        #[serde(default)]
        send_default_pii: bool,
    },
    ScrubEvent {
        #[serde(default)]
        event: serde_json::Value,
        #[serde(default)]
        event_json: Option<String>,
        #[serde(default)]
        send_default_pii: bool,
    },
    SendDefaultPiiFromEnv {
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    BuildSentryInitOptions {
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::ScrubText {
                text,
                send_default_pii,
            } => {
                let scrubbed = scrub_text(text, *send_default_pii)
                    .map_err(|error| as_helper_error("PatternError", error.to_string()))?;
                serde_json::Value::String(scrubbed)
            }
            Input::ScrubEvent {
                event,
                event_json,
                send_default_pii,
            } => decode_and_scrub_event(event, event_json.as_ref(), *send_default_pii)?,
            Input::SendDefaultPiiFromEnv { env } => {
                serde_json::Value::Bool(send_default_pii_from_env(env))
            }
            Input::BuildSentryInitOptions { env } => {
                let options = build_sentry_init_options(env)
                    .map_err(|error| as_helper_error("SettingsError", error.to_string()))?;
                serde_json::json!({
                    "dsn": options.dsn,
                    "traces_sample_rate": options.traces_sample_rate,
                    "sample_rate": options.sample_rate,
                    "send_default_pii": options.send_default_pii,
                    "environment": options.environment,
                    "event_scrubber": {
                        "denylist": options.event_scrubber.denylist,
                        "recursive": options.event_scrubber.recursive,
                    },
                })
            }
        };
        Ok(serde_json::json!({ "result": result }))
    }
}

fn as_helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Box::new(error)
}

fn decode_event(
    event: &serde_json::Value,
    event_json: Option<&String>,
) -> Result<EventValue, Box<crate::helpers::HelperError>> {
    if event_json.is_some() && !event.is_null() {
        return Err(as_helper_error(
            "ValueError",
            "select one event representation".to_owned(),
        ));
    }
    let decoded = event_json.map_or_else(
        || serde_json::from_value(event.clone()),
        |json| serde_json::from_str(json),
    );
    decoded.map_err(|error| as_helper_error("InvalidEvent", error.to_string()))
}

fn decode_and_scrub_event(
    event: &serde_json::Value,
    event_json: Option<&String>,
    send_default_pii: bool,
) -> Result<serde_json::Value, Box<crate::helpers::HelperError>> {
    let typed = decode_event(event, event_json)?;
    let scrubbed = scrub_event(&typed, send_default_pii)
        .map_err(|error| as_helper_error("PatternError", error.to_string()))?;
    serde_json::to_value(scrubbed)
        .map_err(|error| as_helper_error("InvalidEvent", error.to_string()))
}
