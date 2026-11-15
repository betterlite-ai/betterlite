use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::lakera_pii::wire::LakeraResponse;

use super::{HookContext, HookOutcome, prepared_messages};
use crate::lakera_v2::Data;
use crate::lakera_v2::guardrail::LakeraV2Guardrail;
use crate::lakera_v2::scope;

pub async fn during_call(
    guardrail: &LakeraV2Guardrail,
    data: &mut Data,
    context: &mut HookContext<'_>,
) -> Result<HookOutcome, GuardrailFailure> {
    let Some(messages) =
        prepared_messages(guardrail, data, GuardrailEventHooks::DuringCall, context)?
    else {
        return Ok(HookOutcome {
            applied_guardrail: None,
        });
    };
    let (response, _counts) = guardrail
        .call_v2_guard(&messages, data, GuardrailEventHooks::DuringCall, context)
        .await?;
    if response.flagged == Some(true) {
        match guardrail.on_flagged {
            LakeraOnFlagged::InjectSystemMessage => advisory_during_call(&response)?,
            LakeraOnFlagged::Monitor => {
                tracing::warn!(
                    "Lakera Guardrail: Monitoring mode - violation detected but allowing request"
                );
            }
            LakeraOnFlagged::Block => return Err(scope::blocked_failure(Some(&response))?),
        }
    }
    Ok(HookOutcome {
        applied_guardrail: guardrail.base.settings.guardrail_name.clone(),
    })
}

// advisory mode has nothing to inject during during_call, so only PII on the way through blocks
fn advisory_during_call(response: &LakeraResponse) -> Result<(), GuardrailFailure> {
    if scope::breakdown_has_pii_violation(Some(response)) {
        return Err(scope::blocked_failure(Some(response))?);
    }
    tracing::warn!(
        "Lakera Guardrail: Advisory mode has no effect during during_call; violation detected but allowing request"
    );
    Ok(())
}
