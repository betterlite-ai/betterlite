use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::AimError;
use super::input;
use super::request;
use super::settings::{AimGuardrail, AimHook};
use super::wire::{self, AimAction};
use crate::content_inspection::InspectionMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AimOutput {
    Unchanged,
    Redacted { output: String },
    Blocked { detection_message: Option<String> },
}

pub async fn call_aim_guardrail_on_output(
    guardrail: &AimGuardrail,
    request_data: &BTreeMap<String, GuardrailValue>,
    output: &str,
    key_alias: Option<&str>,
) -> Result<AimOutput, AimError> {
    let inspected = input::projected_inspection_messages(request_data);
    let mut messages = inspected.clone();
    messages.push(InspectionMessage {
        role: "assistant".to_owned(),
        content: output.to_owned(),
    });
    let user_email = input::user_email(request_data);
    let call_id = input::text_field(request_data, "litellm_call_id");
    let headers = guardrail.build_headers(AimHook::Output, key_alias, user_email, call_id)?;
    let response = request::analyze(guardrail, &messages, &headers).await?;
    let required = response.required_action.as_ref();
    match wire::action_of(&response) {
        AimAction::Block => Ok(AimOutput::Blocked {
            detection_message: required.and_then(|action| action.detection_message.clone()),
        }),
        AimAction::Anonymize => redacted_output(&response, inspected.len()),
        AimAction::None | AimAction::Monitor | AimAction::Unknown => Ok(AimOutput::Unchanged),
    }
}

fn redacted_output(
    response: &wire::AnalyzeResponse,
    inspected: usize,
) -> Result<AimOutput, AimError> {
    let Some(chat) = response.redacted_chat.as_ref() else {
        return Err(AimError::bad_request(
            "Aim returned no redacted output, so the response can not be rewritten",
        ));
    };
    let redacted = wire::redacted_messages(chat)?;
    if redacted.len() != inspected.saturating_add(1) {
        return Err(AimError::bad_request(
            "Aim returned an invalid redacted output count, so the response can not be rewritten",
        ));
    }
    let Some(last) = redacted.last() else {
        return Err(AimError::bad_request(
            "Aim returned no redacted output, so the response can not be rewritten",
        ));
    };
    if last.content.is_empty() {
        return Err(AimError::bad_request(
            "Aim returned an empty redacted output, so the response can not be rewritten",
        ));
    }
    Ok(AimOutput::Redacted {
        output: last.content.clone(),
    })
}
