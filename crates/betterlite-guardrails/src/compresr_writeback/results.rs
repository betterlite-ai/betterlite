use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::Compressed;
use crate::compresr_recovery::hash::{content_hash, recovery_marker};
use crate::compresr_recovery::store::{Originals, entry_bytes};
use crate::compresr_settings::CompresrGuardrail;
use crate::compresr_transport::{CompressionResult, content::replace_text_in_content};
use crate::generic_guardrail_contracts::GuardrailMessage;

// What the Responses texts mirror needs to replace a text only where that is unambiguous
#[derive(Debug, Default)]
pub(super) struct Replacements {
    pub(super) by_text: BTreeMap<String, String>,
    pub(super) counts: BTreeMap<String, usize>,
    pub(super) ambiguous: BTreeSet<String>,
}

impl Replacements {
    // Two targets with identical text but different compressions cannot be told apart by value, so the text turns ambiguous
    pub(super) fn note(&mut self, original: &str, compressed: &str) {
        match self.by_text.get(original) {
            Some(previous) if previous != compressed => {
                self.ambiguous.insert(original.to_owned());
            }
            _ => {
                self.by_text
                    .insert(original.to_owned(), compressed.to_owned());
            }
        }
        let count = self.counts.entry(original.to_owned()).or_default();
        *count = count.saturating_add(1);
    }
}

#[derive(Debug)]
pub(super) struct Applied {
    pub(super) messages: Vec<GuardrailMessage>,
    pub(super) originals: Originals,
    pub(super) replacements: Replacements,
    pub(super) messages_compressed: i64,
    pub(super) tokens_before: i64,
    pub(super) tokens_after: i64,
    recovery_bytes: usize,
}

impl Applied {
    // A marker is issued only while the store, earlier turns included, stays within the cap, so a stored hash is never evicted by its own call
    fn attach_recovery(
        &mut self,
        existing: &Originals,
        original: &str,
        cap: usize,
    ) -> Option<String> {
        let hash = content_hash(original);
        let already_stored = existing.contains_key(&hash) || self.originals.contains_key(&hash);
        let new_bytes = if already_stored { 0 } else { original.len() };
        let total = self.recovery_bytes.saturating_add(new_bytes);
        if cap != 0 && total > cap {
            return None;
        }
        self.recovery_bytes = total;
        let marker = recovery_marker(&hash);
        self.originals.insert(hash, original.to_owned());
        Some(marker)
    }

    fn write(&mut self, target_idx: usize, original: &str, text: &str, result: &CompressionResult) {
        self.replacements.note(original, text);
        if let Some(message) = self.messages.get_mut(target_idx) {
            let content = message
                .extra
                .get("content")
                .map_or(GuardrailValue::Null, |value| {
                    replace_text_in_content(value, text)
                });
            message.extra.insert("content".to_owned(), content);
        }
        self.tokens_before = self.tokens_before.saturating_add(result.original_tokens);
        self.tokens_after = self.tokens_after.saturating_add(result.compressed_tokens);
    }
}

impl CompresrGuardrail {
    // A result counts only when it is non-empty text that differs from the original; recovery None turns markers off
    pub(super) fn apply_compression_results(
        &self,
        messages: Vec<GuardrailMessage>,
        answer: &Compressed<'_>,
        recovery: Option<&Originals>,
    ) -> Applied {
        let mut applied = Applied {
            messages,
            originals: Originals::new(),
            replacements: Replacements::default(),
            messages_compressed: 0,
            tokens_before: 0,
            tokens_after: 0,
            recovery_bytes: recovery.map_or(0, entry_bytes),
        };
        let planned = answer.plan.indices.iter().zip(&answer.plan.targets);
        for ((&target_idx, target), result) in planned.zip(answer.results) {
            let original = target.context.as_str();
            let Some(compressed) = result
                .compressed_context
                .as_deref()
                .filter(|text| !text.is_empty() && *text != original)
            else {
                continue;
            };
            applied.messages_compressed = applied.messages_compressed.saturating_add(1);
            let mut text = compressed.to_owned();
            if let Some(existing) = recovery
                && let Some(marker) =
                    applied.attach_recovery(existing, original, self.settings.max_bytes_per_call)
            {
                text.push_str(&marker);
            }
            applied.write(target_idx, original, &text, result);
        }
        applied
    }
}
