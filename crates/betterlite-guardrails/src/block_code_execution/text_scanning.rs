use betterlite_config::sections::content_filter_records::{
    BlockCodeAction, BlockCodeExecGuardrailConfig, CodeBlockActionTaken, CodeBlockDetection,
};
use std::future::ready;

use super::block_detection::find_blocks;
use super::escaped_newlines::normalize_escaped_newlines;
use super::intent_detection::{has_execution_intent, has_no_execution_intent};
use super::language_normalization::MASK_PLACEHOLDER;
use crate::generic_guardrail_contracts::GuardrailInputType;

#[derive(Debug, Clone)]
pub struct BlockCodeExecutionGuardrail {
    pub blocked_languages: Option<Vec<String>>,
    pub action: BlockCodeAction,
    pub confidence_threshold: f64,
    pub detect_execution_intent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlockCodeExecutionError {
    #[error("content blocked: {language}")]
    Blocked { language: String },
}

impl From<&BlockCodeExecGuardrailConfig> for BlockCodeExecutionGuardrail {
    fn from(config: &BlockCodeExecGuardrailConfig) -> Self {
        Self {
            blocked_languages: config.blocked_languages.clone(),
            action: config.action,
            confidence_threshold: config.confidence_threshold.0,
            detect_execution_intent: config.detect_execution_intent,
        }
    }
}

impl BlockCodeExecutionGuardrail {
    pub(crate) fn scan_text(
        &self,
        input: &str,
        detections: &mut Vec<CodeBlockDetection>,
        input_type: GuardrailInputType,
    ) -> (String, bool) {
        if input.is_empty() {
            return (String::new(), false);
        }
        let text = normalize_escaped_newlines(input);
        let is_response = input_type == GuardrailInputType::Response;
        if !is_response && self.detect_execution_intent && has_no_execution_intent(&text) {
            return (text, false);
        }
        let block_all = self.blocked_languages.as_ref().is_none_or(Vec::is_empty);
        let blocks = find_blocks(
            &text,
            self.blocked_languages.as_deref(),
            block_all,
            self.confidence_threshold,
        );
        let has_intent =
            !is_response && self.detect_execution_intent && has_execution_intent(&text);
        if blocks.is_empty() {
            if has_intent && self.action == BlockCodeAction::Block {
                detections.push(CodeBlockDetection {
                    r#type: "code_block".to_owned(),
                    language: "execution_request".to_owned(),
                    confidence: 1.0,
                    action_taken: CodeBlockActionTaken::Block,
                    evidence: None,
                    snippet: None,
                });
                return (text, true);
            }
            return (text, false);
        }
        let options = ScanOptions {
            action: self.action,
            enforce_block: is_response || !self.detect_execution_intent || has_intent,
        };
        scan_blocks(&text, blocks, detections, options)
    }

    pub async fn apply_guardrail(
        &self,
        texts: &[String],
        input_type: GuardrailInputType,
    ) -> Result<Vec<String>, BlockCodeExecutionError> {
        ready(()).await;
        let mut detections = Vec::new();
        let mut processed = Vec::with_capacity(texts.len());
        for text in texts {
            let (processed_text, should_raise) = self.scan_text(text, &mut detections, input_type);
            if should_raise {
                let language = detections
                    .iter()
                    .find(|detection| detection.action_taken == CodeBlockActionTaken::Block)
                    .map_or_else(
                        || "unknown".to_owned(),
                        |detection| detection.language.clone(),
                    );
                return Err(BlockCodeExecutionError::Blocked { language });
            }
            processed.push(processed_text);
        }
        Ok(processed)
    }
}

fn scan_blocks(
    text: &str,
    blocks: Vec<super::block_detection::CodeBlock>,
    detections: &mut Vec<CodeBlockDetection>,
    options: ScanOptions,
) -> (String, bool) {
    let mut parts = Vec::new();
    let mut last_end = 0;
    let mut should_raise = false;
    for (start, end, language, _body, confidence, action_taken) in blocks {
        let effective_block = action_taken == CodeBlockActionTaken::Block && options.enforce_block;
        record_detection(
            detections,
            language,
            confidence,
            action_taken,
            effective_block,
        );
        let prefix_length = start.saturating_sub(last_end);
        let block_length = end.saturating_sub(start);
        parts.push(
            text.chars()
                .skip(last_end)
                .take(prefix_length)
                .collect::<String>(),
        );
        if effective_block {
            parts.push(MASK_PLACEHOLDER.to_owned());
            should_raise |= options.action == BlockCodeAction::Block;
        } else {
            parts.push(
                text.chars()
                    .skip(start)
                    .take(block_length)
                    .collect::<String>(),
            );
        }
        last_end = end;
    }
    parts.push(text.chars().skip(last_end).collect::<String>());
    (parts.concat(), should_raise)
}

#[derive(Clone, Copy)]
struct ScanOptions {
    action: BlockCodeAction,
    enforce_block: bool,
}

fn record_detection(
    detections: &mut Vec<CodeBlockDetection>,
    language: String,
    confidence: f64,
    action_taken: CodeBlockActionTaken,
    effective_block: bool,
) {
    detections.push(CodeBlockDetection {
        r#type: "code_block".to_owned(),
        language,
        confidence,
        action_taken: if effective_block {
            CodeBlockActionTaken::Block
        } else {
            action_taken
        },
        evidence: None,
        snippet: None,
    });
}
