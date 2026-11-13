use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::AimError;
use super::input;
use super::request::AimCall;
use super::settings::{AimGuardrail, AimHook};
use super::wire::{self, AimAction};
use crate::content_inspection::InspectionMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AimOutput {
    Unchanged,
    Redacted { output: String },
    Blocked { detection_message: Option<String> },
}

impl AimGuardrail {
    pub async fn call_aim_guardrail_on_output(
        &self,
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
        let user_email = input::user_email(request_data).map(str::to_owned);
        let call_id = input::call_id(request_data).map(str::to_owned);
        let call = AimCall {
            hook: AimHook::Output,
            key_alias,
            user_email: user_email.as_deref(),
            call_id: call_id.as_deref(),
        };
        let response = self.analyze(&messages, &call).await?;
        let required = response.required_action.as_ref();
        match wire::action_of(&response) {
            AimAction::Block => Ok(AimOutput::Blocked {
                detection_message: required.and_then(|action| action.detection_message.clone()),
            }),
            AimAction::Anonymize => redacted_output(&response, inspected.len()),
            AimAction::None | AimAction::Monitor | AimAction::Unknown => Ok(AimOutput::Unchanged),
        }
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
