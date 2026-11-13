use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::scoped_indices::filter_messages_by_skip_flags;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::lakera_pii::masking::{is_only_pii_violation, mask_pii_in_messages};
use crate::lakera_pii::wire::{LakeraResponse, Message};

use super::{HookContext, HookOutcome};
use crate::lakera_v2::Data;
use crate::lakera_v2::error::LakeraV2Error;
use crate::lakera_v2::guardrail::LakeraV2Guardrail;
use crate::lakera_v2::scope;

struct Masked<'masked> {
    response: &'masked mut Data,
    lakera_response: &'masked LakeraResponse,
    messages: &'masked [Message],
    original_len: usize,
    choice_indices: &'masked [usize],
}

impl LakeraV2Guardrail {
    pub async fn post_call(
        &self,
        data: &mut Data,
        response: &mut Data,
        context: &mut HookContext<'_>,
    ) -> Result<HookOutcome, GuardrailFailure> {
        if !self.base.should_run_guardrail(
            data,
            GuardrailEventHooks::PostCall,
            context.suppressed,
        )? {
            return Ok(HookOutcome {
                applied_guardrail: None,
            });
        }
        let request_messages = match data.get("messages") {
            Some(GuardrailValue::Array(messages)) => messages.clone(),
            _ => Vec::new(),
        };
        let (original, _changed) = filter_messages_by_skip_flags(&request_messages, &context.scope);
        let (assistant, choice_indices) = response_messages(response);
        let mut values = original.clone();
        values.extend(assistant);
        let messages = scope::values_to_messages(&values)?;
        let (lakera_response, _counts) = self
            .call_v2_guard(&messages, data, GuardrailEventHooks::PostCall, context)
            .await?;
        if lakera_response.flagged == Some(true) {
            self.enforce(Masked {
                response,
                lakera_response: &lakera_response,
                messages: &messages,
                original_len: original.len(),
                choice_indices: &choice_indices,
            })?;
        }
        Ok(HookOutcome {
            applied_guardrail: self.base.settings.guardrail_name.clone(),
        })
    }

    fn enforce(&self, masked: Masked<'_>) -> Result<(), GuardrailFailure> {
        let Masked {
            response,
            lakera_response,
            messages,
            original_len,
            choice_indices,
        } = masked;
        if is_only_pii_violation(Some(lakera_response)) {
            let mut counts = BTreeMap::new();
            let masked_messages =
                mask_pii_in_messages(messages, Some(lakera_response), &mut counts);
            let assistant = masked_messages.get(original_len..).unwrap_or_default();
            for (position, message) in assistant.iter().enumerate() {
                let Some(choice) = choice_indices.get(position) else {
                    continue;
                };
                write_choice_content(response, *choice, scope::content_text(message)?)?;
            }
            return Ok(());
        }
        match self.on_flagged {
            LakeraOnFlagged::Monitor | LakeraOnFlagged::InjectSystemMessage => {
                tracing::warn!(
                    on_flagged = on_flagged_name(self.on_flagged),
                    "Lakera Guardrail: Post-call violation detected - allowing response"
                );
            }
            LakeraOnFlagged::Block => {
                return Err(scope::blocked_failure(Some(lakera_response))?);
            }
        }
        Ok(())
    }
}

fn response_messages(response: &Data) -> (Vec<GuardrailValue>, Vec<usize>) {
    let mut messages = Vec::new();
    let mut indices = Vec::new();
    let Some(GuardrailValue::Array(choices)) = response.get("choices") else {
        return (messages, indices);
    };
    for (index, choice) in choices.iter().enumerate() {
        let GuardrailValue::Object(fields) = choice else {
            continue;
        };
        let Some(GuardrailValue::Object(message)) = fields.get("message") else {
            continue;
        };
        let Some(GuardrailValue::Text(role)) = message.get("role") else {
            continue;
        };
        let Some(GuardrailValue::Text(content)) = message.get("content") else {
            continue;
        };
        if role.is_empty() || content.is_empty() {
            continue;
        }
        messages.push(GuardrailValue::Object(BTreeMap::from([
            ("role".to_owned(), GuardrailValue::Text(role.clone())),
            ("content".to_owned(), GuardrailValue::Text(content.clone())),
        ])));
        indices.push(index);
    }
    (messages, indices)
}

fn write_choice_content(
    response: &mut Data,
    choice: usize,
    content: String,
) -> Result<(), LakeraV2Error> {
    let Some(GuardrailValue::Array(choices)) = response.get_mut("choices") else {
        return Err(LakeraV2Error::ResponseShape { at: "choices" });
    };
    let Some(GuardrailValue::Object(carrier)) = choices.get_mut(choice) else {
        return Err(LakeraV2Error::ResponseShape { at: "choices[]" });
    };
    let Some(GuardrailValue::Object(message)) = carrier.get_mut("message") else {
        return Err(LakeraV2Error::ResponseShape {
            at: "choices[].message",
        });
    };
    message.insert("content".to_owned(), GuardrailValue::Text(content));
    Ok(())
}

const fn on_flagged_name(flagged: LakeraOnFlagged) -> &'static str {
    match flagged {
        LakeraOnFlagged::Block => "block",
        LakeraOnFlagged::Monitor => "monitor",
        LakeraOnFlagged::InjectSystemMessage => "inject_system_message",
    }
}
