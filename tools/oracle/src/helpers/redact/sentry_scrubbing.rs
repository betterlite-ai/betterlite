use std::collections::BTreeMap;

use betterlite_redact::sentry::scrub::{EventValue, scrub_event, scrub_text};
use betterlite_redact::sentry::settings::{
    SettingsError, build_sentry_init_options, send_default_pii_from_env,
};

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
                let scrubbed = scrub_text(text, *send_default_pii).map_err(|error| pattern_error(&error))?;
                serde_json::Value::String(scrubbed)
            }
            Input::ScrubEvent {
                event,
                event_json,
                send_default_pii,
            } => {
                if event_json.is_some() && !event.is_null() {
                    return Err(invalid_event_selection());
                }
                let typed: EventValue = match event_json {
                    Some(json) => {
                        serde_json::from_str(json).map_err(|error| invalid_event(&error))?
                    }
                    None => serde_json::from_value(event.clone())
                        .map_err(|error| invalid_event(&error))?,
                };
                let scrubbed = scrub_event(&typed, *send_default_pii)
                    .map_err(|error| pattern_error(&error))?;
                serde_json::to_value(scrubbed).map_err(|error| invalid_event(&error))?
            }
            Input::SendDefaultPiiFromEnv { env } => {
                serde_json::Value::Bool(send_default_pii_from_env(env))
            }
            Input::BuildSentryInitOptions { env } => {
                let options =
                    build_sentry_init_options(env).map_err(|error| settings_error(&error))?;
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

fn pattern_error(error: &betterlite_redact::sentry::pattern::PatternError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "PatternError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn settings_error(error: &SettingsError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "SettingsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn invalid_event_selection() -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: "select one event representation".to_owned(),
    })
}

fn invalid_event(error: &serde_json::Error) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidEvent".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
