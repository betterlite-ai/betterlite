use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailLogTarget;
use crate::guardrail_message_scope::scoped_indices::{ScopeFlags, filter_messages_by_skip_flags};
use crate::guardrail_records::invocation::GuardrailInvocation;
use crate::guardrail_records::record::GuardrailSpanSink;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::lakera_pii::wire::Message;

use super::Data;
use super::guardrail::LakeraV2Guardrail;
use super::scope;

pub mod during_call;
pub mod post_call;
pub mod pre_call;

pub struct HookContext<'context> {
    pub scope: ScopeFlags,
    pub suppressed: &'context BTreeSet<String>,
    pub invocation: &'context mut GuardrailInvocation,
    pub sink: &'context dyn GuardrailSpanSink,
    pub log_target: Option<&'context mut GuardrailLogTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookOutcome {
    pub applied_guardrail: Option<String>,
}

impl LakeraV2Guardrail {
    fn prepared_messages(
        &self,
        data: &Data,
        event: GuardrailEventHooks,
        context: &HookContext<'_>,
    ) -> Result<Option<Vec<Message>>, GuardrailFailure> {
        if !self
            .base
            .should_run_guardrail(data, event, context.suppressed)?
        {
            tracing::debug!("Lakera AI: not running guardrail. Guardrail is disabled.");
            return Ok(None);
        }
        let inspection = scope::inspection_messages(data);
        if inspection.is_empty() {
            tracing::warn!("Lakera AI: not running guardrail. No inspectable text in data");
            return Ok(None);
        }
        let values = scope::inspection_values(&inspection);
        let (kept, _changed) = filter_messages_by_skip_flags(&values, &context.scope);
        if kept.is_empty() {
            tracing::warn!(
                "Lakera AI: not running guardrail. All inspectable text was excluded by skip_system_message_in_guardrail/skip_tool_message_in_guardrail"
            );
            return Ok(None);
        }
        Ok(Some(scope::values_to_messages(&kept)?))
    }
}
