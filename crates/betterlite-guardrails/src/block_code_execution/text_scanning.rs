#[cfg(test)]
mod tests;

use betterlite_config::sections::content_filter_records::{
    BlockCodeAction, CodeBlockActionTaken, CodeBlockDetection,
};

use super::block_detection::{BlockDetector, CodeBlock};
use super::escaped_newlines::normalize_escaped_newlines;
use super::intent_detection::{has_execution_intent, has_no_execution_intent};
use super::language_normalization::MASK_PLACEHOLDER;
use crate::generic_guardrail_contracts::GuardrailInputType;

#[derive(Debug, Clone)]
pub struct CodeBlockScanner {
    pub detector: BlockDetector,
    pub action: BlockCodeAction,
    pub detect_execution_intent: bool,
}

impl CodeBlockScanner {
    pub fn scan_text(
        &self,
        input: &str,
        detections: &mut Vec<CodeBlockDetection>,
        input_type: GuardrailInputType,
    ) -> (String, bool) {
        let text = normalize_escaped_newlines(input);
        // LLM output carries no user intent, so the heuristics apply to requests only
        let checks_intent =
            input_type == GuardrailInputType::Request && self.detect_execution_intent;
        if checks_intent && has_no_execution_intent(&text) {
            return (text, false);
        }
        let blocks = self.detector.find_blocks(&text);
        let wants_execution = checks_intent && has_execution_intent(&text);
        if blocks.is_empty() {
            let should_raise = wants_execution && self.action == BlockCodeAction::Block;
            if should_raise {
                detections.push(CodeBlockDetection {
                    r#type: "code_block".to_owned(),
                    language: "execution_request".to_owned(),
                    confidence: 1.0,
                    action_taken: CodeBlockActionTaken::Block,
                    evidence: None,
                    snippet: None,
                });
            }
            return (text, should_raise);
        }
        self.mask_blocks(&text, blocks, !checks_intent || wants_execution, detections)
    }

    fn mask_blocks(
        &self,
        text: &str,
        blocks: Vec<CodeBlock>,
        enforce_block: bool,
        detections: &mut Vec<CodeBlockDetection>,
    ) -> (String, bool) {
        let mut output = String::with_capacity(text.len());
        let mut remaining = text.chars();
        let mut last_end = 0;
        let mut should_raise = false;
        for block in blocks {
            let effective_block =
                block.action_taken == CodeBlockActionTaken::Block && enforce_block;
            output.extend(
                remaining
                    .by_ref()
                    .take(block.start.saturating_sub(last_end)),
            );
            let original: String = remaining
                .by_ref()
                .take(block.end.saturating_sub(block.start))
                .collect();
            if effective_block {
                output.push_str(MASK_PLACEHOLDER);
                should_raise |= self.action == BlockCodeAction::Block;
            } else {
                output.push_str(&original);
            }
            last_end = block.end;
            // confidence is 0.0, 0.5 or 1.0, so LiteLLM's round(confidence, 2) changes nothing
            detections.push(CodeBlockDetection {
                r#type: "code_block".to_owned(),
                language: block.language,
                confidence: block.confidence,
                action_taken: block.action_taken,
                evidence: None,
                snippet: None,
            });
        }
        output.extend(remaining);
        (output, should_raise)
    }
}
