use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::{AimError, CONTENT_POLICY_VIOLATION};
use super::input;
use super::settings::{AimGuardrail, AimHook};
use super::wire::{self, AimAction, AnalyzeRequest, AnalyzeResponse};
use crate::content_inspection::{
    InspectionMessage, apply_redacted_messages_back, has_non_string_content, is_string_batch_input,
};

pub struct AimCall<'context> {
    pub hook: AimHook,
    pub key_alias: Option<&'context str>,
    pub user_email: Option<&'context str>,
    pub call_id: Option<&'context str>,
}

impl AimGuardrail {
    pub async fn call_aim_guardrail(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        hook: AimHook,
        key_alias: Option<&str>,
    ) -> Result<(), AimError> {
        let messages = input::projected_inspection_messages(data);
        let user_email = input::user_email(data).map(str::to_owned);
        let call_id = input::call_id(data).map(str::to_owned);
        let call = AimCall {
            hook,
            key_alias,
            user_email: user_email.as_deref(),
            call_id: call_id.as_deref(),
        };
        let response = self.analyze(&messages, &call).await?;
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
        &self,
        messages: &[InspectionMessage],
        call: &AimCall<'_>,
    ) -> Result<AnalyzeResponse, AimError> {
        let headers =
            self.build_headers(call.hook, call.key_alias, call.user_email, call.call_id)?;
        let body = serde_json::to_vec(&AnalyzeRequest { messages })
            .map_err(|source| AimError::RequestJson(source))?;
        let url = format!("{}/fw/v1/analyze", self.api_base);
        let response = self
            .poster
            .post(&url, &headers, &body, self.timeout)
            .await?;
        if !response.status.is_success() {
            return Err(AimError::HttpStatus(response.status));
        }
        serde_json::from_slice(&response.body).map_err(|source| AimError::ResponseJson(source))
    }
}

fn anonymize(
    response: &AnalyzeResponse,
    data: &mut BTreeMap<String, GuardrailValue>,
) -> Result<(), AimError> {
    let Some(chat) = response
        .redacted_chat
        .as_ref()
        .filter(|chat| chat.all_redacted_messages.is_some())
    else {
        return Ok(());
    };
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
