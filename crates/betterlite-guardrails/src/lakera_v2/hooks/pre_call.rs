use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::guardrail_translation::LakeraOnFlagged;

use crate::content_inspection::has_non_string_content;
use crate::guardrail_base::advisory::inject_advisory_message;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::scoped_indices::ScopeFlags;
use crate::lakera_pii::masking::{is_only_pii_violation, mask_pii_in_messages};
use crate::lakera_pii::wire::{LakeraResponse, Message};

use super::{HookContext, HookOutcome};
use crate::lakera_v2::Data;
use crate::lakera_v2::guardrail::LakeraV2Guardrail;
use crate::lakera_v2::scope;

struct Flagged<'flagged> {
    data: &'flagged mut Data,
    messages: &'flagged [Message],
    response: &'flagged LakeraResponse,
    counts: &'flagged mut BTreeMap<String, u64>,
    multimodal: bool,
    scope: ScopeFlags,
}

impl LakeraV2Guardrail {
    pub async fn pre_call(
        &self,
        data: &mut Data,
        context: &mut HookContext<'_>,
    ) -> Result<HookOutcome, GuardrailFailure> {
        let Some(messages) = self.prepared_messages(data, GuardrailEventHooks::PreCall, context)?
        else {
            return Ok(HookOutcome {
                applied_guardrail: None,
            });
        };
        let multimodal = has_non_string_content(data)
            || scope::has_combined_input(data)
            || scope::has_responses_instructions(data, context.scope);
        let (response, mut counts) = self
            .call_v2_guard(&messages, data, GuardrailEventHooks::PreCall, context)
            .await?;
        if response.flagged == Some(true) {
            self.enforce(Flagged {
                data,
                messages: &messages,
                response: &response,
                counts: &mut counts,
                multimodal,
                scope: context.scope,
            })?;
        }
        Ok(HookOutcome {
            applied_guardrail: self.base.settings.guardrail_name.clone(),
        })
    }

    fn enforce(&self, flagged: Flagged<'_>) -> Result<(), GuardrailFailure> {
        let pii_only = is_only_pii_violation(Some(flagged.response));
        if pii_only && !flagged.multimodal {
            mask_back(
                flagged.data,
                flagged.messages,
                flagged.response,
                flagged.counts,
                flagged.scope,
            )?;
            tracing::debug!("Lakera AI: Masked PII in messages instead of blocking request");
            return Ok(());
        }
        match self.on_flagged {
            LakeraOnFlagged::InjectSystemMessage => self.inject_advisory(flagged),
            LakeraOnFlagged::Monitor => self.monitor(flagged, pii_only),
            LakeraOnFlagged::Block => Err(scope::blocked_failure(Some(flagged.response))?),
        }
    }

    fn inject_advisory(&self, flagged: Flagged<'_>) -> Result<(), GuardrailFailure> {
        let Flagged {
            data,
            messages,
            response,
            counts,
            multimodal,
            scope: flags,
        } = flagged;
        let has_pii = scope::breakdown_has_pii_violation(Some(response));
        if has_pii && multimodal {
            return Err(scope::blocked_failure(Some(response))?);
        }
        if has_pii {
            mask_back(data, messages, response, counts, flags)?;
        }
        let advisory = self.build_advisory_message(Some(response))?;
        if inject_advisory_message(data, &advisory) {
            tracing::warn!(
                "Lakera Guardrail: Advisory mode - violation detected, advisory system message appended"
            );
            return Ok(());
        }
        Err(scope::blocked_failure(Some(response))?)
    }

    fn monitor(&self, flagged: Flagged<'_>, pii_only: bool) -> Result<(), GuardrailFailure> {
        let Flagged {
            data,
            messages,
            response,
            counts,
            scope: flags,
            ..
        } = flagged;
        if !pii_only {
            tracing::warn!(
                "Lakera Guardrail: Monitoring mode - violation detected but allowing request"
            );
            return Ok(());
        }
        let masked = scope::mask_instructions_pii(
            data,
            messages,
            Some(response),
            counts,
            flags,
        )?;
        if masked {
            tracing::warn!(
                "Lakera Guardrail: Monitoring mode - PII detected, masked in place and allowing request"
            );
            return Ok(());
        }
        let reason = scope::unmaskable_reason(data, Some(response), flags).unwrap_or_default();
        tracing::error!(
            reason = reason,
            "Lakera Guardrail: Monitoring mode - PII detected but NOT masked, forwarding unredacted content to the model"
        );
        Ok(())
    }
}

fn mask_back(
    data: &mut Data,
    messages: &[Message],
    response: &LakeraResponse,
    counts: &mut BTreeMap<String, u64>,
    scope: ScopeFlags,
) -> Result<(), GuardrailFailure> {
    let redacted = mask_pii_in_messages(messages, Some(response), counts);
    scope::apply_redacted_back(data, &redacted, scope)?;
    Ok(())
}
