use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;

use crate::guardrail_base::failure::GuardrailFailure;

use super::{HookContext, HookOutcome};
use crate::lakera_v2::Data;
use crate::lakera_v2::guardrail::LakeraV2Guardrail;
use crate::lakera_v2::scope;

impl LakeraV2Guardrail {
    pub async fn during_call(
        &self,
        data: &mut Data,
        context: &mut HookContext<'_>,
    ) -> Result<HookOutcome, GuardrailFailure> {
        let Some(messages) =
            self.prepared_messages(data, GuardrailEventHooks::DuringCall, context)?
        else {
            return Ok(HookOutcome {
                applied_guardrail: None,
            });
        };
        let (response, _counts) = self
            .call_v2_guard(&messages, data, GuardrailEventHooks::DuringCall, context)
            .await?;
        if response.flagged == Some(true) {
            match self.on_flagged {
                LakeraOnFlagged::InjectSystemMessage => {
                    if scope::breakdown_has_pii_violation(Some(&response)) {
                        return Err(scope::blocked_failure(Some(&response))?);
                    }
                    tracing::warn!(
                        "Lakera Guardrail: Advisory mode has no effect during during_call; violation detected but allowing request"
                    );
                }
                LakeraOnFlagged::Monitor => {
                    tracing::warn!(
                        "Lakera Guardrail: Monitoring mode - violation detected but allowing request"
                    );
                }
                LakeraOnFlagged::Block => {
                    return Err(scope::blocked_failure(Some(&response))?);
                }
            }
        }
        Ok(HookOutcome {
            applied_guardrail: self.base.settings.guardrail_name.clone(),
        })
    }
}
