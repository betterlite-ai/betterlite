use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::header::HeaderMap;

use super::error::{AimError, CONTENT_POLICY_VIOLATION};
use super::input;
use super::settings::{AimGuardrail, AimHook};
use super::wire::{self, AimAction, AnalyzeRequest, AnalyzeResponse};
use crate::content_inspection::{
    InspectionMessage, apply_redacted_messages_back, has_non_string_content, is_string_batch_input,
};

pub async fn call_aim_guardrail(
    guardrail: &AimGuardrail,
    data: &mut BTreeMap<String, GuardrailValue>,
    hook: AimHook,
    key_alias: Option<&str>,
) -> Result<(), AimError> {
    let messages = input::projected_inspection_messages(data);
    let user_email = input::user_email(data);
    let call_id = input::text_field(data, "litellm_call_id");
    let headers = guardrail.build_headers(hook, key_alias, user_email, call_id)?;
    let response = analyze(guardrail, &messages, &headers).await?;
    match wire::action_of(&response) {
        AimAction::None | AimAction::Monitor => Ok(()),
        AimAction::Block => Err(AimError::Rejected {
            message: response
                .required_action
                .as_ref()
                .and_then(|action| action.detection_message.clone())
                .unwrap_or_else(|| {
                    "Aim blocked the request under its guardrail policies".to_owned()
                }),
            code: CONTENT_POLICY_VIOLATION,
        }),
        AimAction::Anonymize => anonymize(&response, data),
        AimAction::Unknown => {
            if let Some(action_type) = response
                .required_action
                .as_ref()
                .and_then(|action| action.action_type.as_deref())
            {
                tracing::error!(action_type, "Aim: unsupported action");
            }
            Ok(())
        }
    }
}

pub(crate) async fn analyze(
    guardrail: &AimGuardrail,
    messages: &[InspectionMessage],
    headers: &HeaderMap,
) -> Result<AnalyzeResponse, AimError> {
    let body = serde_json::to_vec(&AnalyzeRequest { messages }).map_err(AimError::RequestJson)?;
    let url = format!("{}/fw/v1/analyze", guardrail.api_base);
    let response = guardrail
        .poster
        .post(&url, headers, &body, guardrail.timeout)
        .await?;
    if !response.status.is_success() {
        return Err(AimError::HttpStatus(response.status));
    }
    serde_json::from_slice(&response.body).map_err(AimError::ResponseJson)
}

fn anonymize(
    response: &AnalyzeResponse,
    data: &mut BTreeMap<String, GuardrailValue>,
) -> Result<(), AimError> {
    let Some(chat) = response.redacted_chat.as_ref() else {
        return Ok(());
    };
    // LiteLLM returns the request untouched only for a missing, null or empty redacted_chat
    if matches!(chat, GuardrailValue::Object(fields) if fields.is_empty()) {
        return Ok(());
    }
    // the redacted batch is text only, so rewriting a multimodal request would drop its image and audio parts
    if has_non_string_content(data) && !is_string_batch_input(data) {
        return Err(AimError::bad_request(
            "Aim anonymize can not rewrite multimodal input without dropping its non-text parts",
        ));
    }
    let redacted = wire::redacted_messages(chat)?;
    if redacted.len() != input::projected_inspection_messages(data).len() {
        return Err(AimError::bad_request(
            "Aim returned a redacted batch of a different size than the inspected input",
        ));
    }
    let messages: Vec<GuardrailValue> = redacted.iter().map(input::redacted_value).collect();
    if !apply_redacted_messages_back(data, &messages) {
        return Err(AimError::bad_request(
            "Aim returned a redacted batch of a different size than the inspected input",
        ));
    }
    Ok(())
}
