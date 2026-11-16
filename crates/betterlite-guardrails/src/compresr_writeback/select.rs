use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::compresr_settings::{CompresrGuardrail, Compressible};
use crate::compresr_transport::CompressTarget;
use crate::compresr_transport::query::{fallback_query, query_for_target};
use crate::content_text::{content_to_text, is_all_text_parts};
use crate::generic_guardrail_contracts::GuardrailMessage;

#[derive(Debug, Default)]
pub(super) struct Plan {
    pub(super) indices: Vec<usize>,
    pub(super) targets: Vec<CompressTarget>,
}

impl CompresrGuardrail {
    // Indices, in order, of the messages whose text should be compressed
    pub(super) fn select_targets(
        &self,
        messages: &[GuardrailMessage],
        query_idx: Option<usize>,
    ) -> Vec<usize> {
        let mut targets = Vec::new();
        for (idx, message) in messages.iter().enumerate() {
            if self.wants_role(message.role.as_str(), query_idx == Some(idx))
                && self.is_long_text(message.extra.get("content"))
            {
                targets.push(idx);
            }
        }
        targets
    }

    // The last user message is the query, so it only joins when compress_last_user says so
    fn wants_role(&self, role: &str, is_query: bool) -> bool {
        let wanted = &self.settings.targets;
        if is_query && !wanted.contains(&Compressible::LastUser) {
            return false;
        }
        match role {
            "tool" | "function" => wanted.contains(&Compressible::ToolOutputs),
            "system" => wanted.contains(&Compressible::System),
            "user" => is_query || wanted.contains(&Compressible::History),
            _ => false,
        }
    }

    // A part list with a non-text part keeps its content, since one compressed string cannot be written back across it
    fn is_long_text(&self, content: Option<&GuardrailValue>) -> bool {
        if matches!(content, Some(GuardrailValue::Array(_)))
            && !content.is_some_and(is_all_text_parts)
        {
            return false;
        }
        let length = content.map_or(0, |value| content_to_text(value).chars().count());
        length >= self.settings.min_chars_to_compress
    }

    // Each selected message pairs with the query it is compressed against, and a target without a query stays as it is
    pub(super) fn plan_targets(&self, messages: &[GuardrailMessage]) -> Plan {
        let (fallback, query_idx) = fallback_query(messages);
        let mut plan = Plan::default();
        for idx in self.select_targets(messages, query_idx) {
            let query = query_for_target(messages, idx, &fallback);
            if query.trim().is_empty() {
                continue;
            }
            let context = messages
                .get(idx)
                .and_then(|message| message.extra.get("content"))
                .map_or_else(String::new, content_to_text);
            plan.indices.push(idx);
            plan.targets.push(CompressTarget { context, query });
        }
        plan
    }
}
