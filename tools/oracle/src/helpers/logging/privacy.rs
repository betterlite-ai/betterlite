use crate::compare::helper::HelperError;
use crate::helpers::{Helper, HelperResult};
use betterlite_config::sections::logging_privacy::Settings;
use betterlite_logging::logging_privacy::request::{Decision, Flag, PrivacyRequest};
use betterlite_logging::logging_privacy::snapshot::{callback_snapshot, redact_user_api_key_info};
use betterlite_logging::logging_privacy::{CallbackMetadataError, PrivacyContext};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    #[default]
    Snapshot,
    Metadata,
    Copies,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Flags {
    global: bool,
    redact_key_info: bool,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    action: Action,
    #[serde(flatten)]
    flags: Flags,
    dynamic: Option<Flag>,
    headers: IndexMap<String, Flag>,
    request: PrivacyRequest,
    inherited: bool,
    payload: Json,
    key_info: Json,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        if matches!(input.action, Action::Metadata) {
            let params = input
                .request
                .dynamic_params(PrivacyContext {
                    message_logging_disabled: input.inherited,
                })
                .map_err(|error| failure(&error))?;
            return Ok(serde_json::json!(params));
        }
        snapshots(input)
    }
}

fn decision(input: &Input) -> Result<Decision, Box<HelperError>> {
    let mut request = PrivacyRequest::default();
    request
        .direct
        .turn_off_message_logging
        .clone_from(&input.dynamic);
    request.direct.headers.clone_from(&input.headers);
    request
        .decide(
            Settings {
                turn_off_message_logging: input.flags.global,
                redact_user_api_key_info: input.flags.redact_key_info,
            },
            PrivacyContext {
                message_logging_disabled: input.inherited,
            },
            true,
        )
        .map_err(|error| failure(&error))
}

fn snapshots(input: &Input) -> HelperResult {
    let privacy = decision(input)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| failure(&CallbackMetadataError::Redactor { source }))?;
    let snapshot = callback_snapshot(input.payload.clone(), privacy, &redactor);
    if matches!(input.action, Action::Copies) {
        let public = callback_snapshot(
            input.payload.clone(),
            Decision {
                content: false,
                key_info: false,
            },
            &redactor,
        );
        return Ok(serde_json::json!({"private":snapshot,"public":public,"source":input.payload}));
    }
    let key_info = redact_user_api_key_info(&input.key_info, privacy.key_info);
    Ok(serde_json::json!({"should_redact":privacy.content,"snapshot":snapshot,"key_info":key_info}))
}

fn failure(error: &CallbackMetadataError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "PrivacyError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl Default for Input {
    fn default() -> Self {
        Self {
            action: Action::default(),
            flags: Flags::default(),
            dynamic: None,
            headers: IndexMap::new(),
            request: PrivacyRequest::default(),
            inherited: false,
            payload: Json::Null,
            key_info: Json::Null,
        }
    }
}
