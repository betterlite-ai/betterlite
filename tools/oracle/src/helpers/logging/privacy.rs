use betterlite_config::sections::logging_privacy::Settings;
use betterlite_logging::logging_privacy::request::{Flag, PrivacyRequest};
use betterlite_logging::logging_privacy::{
    PrivacyContext, callback_snapshot, redact_user_api_key_info,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    #[default]
    Snapshot,
    Metadata,
    Copies,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    action: Action,
    global: bool,
    dynamic: Option<Flag>,
    headers: IndexMap<String, Flag>,
    request: PrivacyRequest,
    inherited: bool,
    payload: Option<Json>,
    key_info: Option<Json>,
    redact_key_info: bool,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        if matches!(input.action, Action::Metadata) {
            let mut params = input.request.dynamic_params().map_err(failure)?;
            if input.inherited {
                params.insert("turn_off_message_logging".to_owned(), Json::Bool(true));
            }
            return Ok(serde_json::json!(params));
        }
        let mut request = PrivacyRequest::default();
        request.direct.turn_off_message_logging = input.dynamic.clone();
        request.direct.headers = input.headers.clone();
        let decision = request
            .decide(
                Settings {
                    turn_off_message_logging: input.global,
                    redact_user_api_key_info: input.redact_key_info,
                },
                PrivacyContext {
                    message_logging_disabled: input.inherited,
                },
                true,
            )
            .map_err(failure)?;
        let redactor = betterlite_redact::secret::Redactor::new(
            betterlite_redact::secret::MINIMUM_CUSTOM_KEY_LENGTH,
        )
        .map_err(|source| {
            failure(betterlite_logging::logging_privacy::CallbackMetadataError::Redactor { source })
        })?;
        let snapshot = callback_snapshot(
            input.payload.clone().unwrap_or(Json::Null),
            decision,
            &redactor,
        );
        if matches!(input.action, Action::Copies) {
            let public = callback_snapshot(
                input.payload.clone().unwrap_or(Json::Null),
                betterlite_logging::logging_privacy::request::Decision {
                    content: false,
                    key_info: false,
                },
                &redactor,
            );
            return Ok(
                serde_json::json!({"private":snapshot,"public":public,"source":input.payload}),
            );
        }
        let key_info = input.key_info.as_ref().map_or(Json::Null, |metadata| {
            redact_user_api_key_info(metadata, decision.key_info)
        });
        Ok(
            serde_json::json!({"should_redact":decision.content,"snapshot":snapshot,"key_info":key_info}),
        )
    }
}

fn failure(
    error: betterlite_logging::logging_privacy::CallbackMetadataError,
) -> Box<crate::compare::helper::HelperError> {
    Box::new(crate::compare::helper::HelperError {
        class: "PrivacyError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
