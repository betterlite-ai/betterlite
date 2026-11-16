use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::error::ThirdPartyHookError;
use crate::aim::error::{AimError, CONTENT_POLICY_VIOLATION};
use crate::aim::output::{AimOutput, call_aim_guardrail_on_output};
use crate::aim::request::call_aim_guardrail;
use crate::aim::settings::{AimGuardrail, AimHook};
use crate::content_inspection::NON_CONVERSATIONAL_CALL_TYPES;
use crate::lasso_masking::wire::ModelResponseShape;

// Which request-side hook runs: LiteLLM's pre_call and moderation differ only in the hook they send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AimRequestHook {
    PreCall,
    Moderation,
}

impl AimRequestHook {
    const fn aim_hook(self) -> AimHook {
        match self {
            Self::PreCall => AimHook::PreCall,
            Self::Moderation => AimHook::Moderation,
        }
    }
}

// embeddings carry documents being indexed, not a conversation; Aim inspects them only when asked.
// pre_call may rewrite the request in place; the during-call dispatch cannot rewrite, so moderation
// inspects a copy and drops the anonymized result.
pub async fn hook(
    guardrail: &AimGuardrail,
    data: &mut BTreeMap<String, GuardrailValue>,
    hook: AimRequestHook,
    call_type: &str,
    key_alias: Option<&str>,
) -> Result<(), ThirdPartyHookError> {
    if NON_CONVERSATIONAL_CALL_TYPES.contains(&call_type) && !guardrail.inspect_embeddings {
        tracing::debug!(call_type, "Aim: skipping non-conversational call type");
        return Ok(());
    }
    match hook {
        AimRequestHook::PreCall => {
            call_aim_guardrail(guardrail, data, hook.aim_hook(), key_alias).await?;
        }
        AimRequestHook::Moderation => {
            let mut inspected = data.clone();
            call_aim_guardrail(guardrail, &mut inspected, hook.aim_hook(), key_alias).await?;
        }
    }
    Ok(())
}

pub async fn post_call_success(
    guardrail: &AimGuardrail,
    data: &BTreeMap<String, GuardrailValue>,
    response: &mut ModelResponseShape,
    key_alias: Option<&str>,
) -> Result<(), ThirdPartyHookError> {
    let mut indexes = Vec::new();
    let mut outputs = Vec::new();
    for (index, choice) in response.choices.iter().enumerate() {
        let Some(message) = choice.message.as_ref() else {
            continue;
        };
        indexes.push(index);
        outputs.push(message.content.clone().unwrap_or_default());
    }
    if indexes.is_empty() {
        return Ok(());
    }
    let results = inspect_choices(guardrail, data, &outputs, key_alias).await;
    apply_results(response, &indexes, results)
}

// all choices are inspected at the same time; gather's return_exceptions keeps every call running
async fn inspect_choices(
    guardrail: &AimGuardrail,
    data: &BTreeMap<String, GuardrailValue>,
    outputs: &[String],
    key_alias: Option<&str>,
) -> Vec<Result<AimOutput, AimError>> {
    let mut calls = Vec::with_capacity(outputs.len());
    for output in outputs {
        calls.push(call_aim_guardrail_on_output(
            guardrail, data, output, key_alias,
        ));
    }
    futures_util::future::join_all(calls).await
}

// the calls already finished; their results are read in choice order, so the first blocking choice decides
fn apply_results(
    response: &mut ModelResponseShape,
    indexes: &[usize],
    results: Vec<Result<AimOutput, AimError>>,
) -> Result<(), ThirdPartyHookError> {
    for (index, result) in indexes.iter().zip(results) {
        match result? {
            AimOutput::Unchanged => {}
            AimOutput::Redacted { output } => {
                if let Some(choice) = response.choices.get_mut(*index)
                    && let Some(message) = choice.message.as_mut()
                {
                    message.content = Some(output);
                }
            }
            AimOutput::Blocked { detection_message } => {
                if let Some(message) = detection_message.filter(|text| !text.is_empty()) {
                    return Err(ThirdPartyHookError::from(AimError::Rejected {
                        message,
                        code: CONTENT_POLICY_VIOLATION,
                    }));
                }
            }
        }
    }
    Ok(())
}
