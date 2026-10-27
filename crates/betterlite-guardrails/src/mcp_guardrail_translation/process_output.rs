use super::call_tool_result::CallToolResult;
use super::error::{ContentBlocked, NestedPayload, ScanTarget, TranslationError};
use super::guardrail::{ApplyGuardrail, GuardrailInputs};
use super::nesting_depth::{JsonScan, scan_json};
use super::result_mutation::mask_result;
use super::text_count_mismatch::masked_texts;
use crate::generic_guardrail_contracts::GuardrailInputType;

mod tests;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputScan {
    pub text_blocks: Vec<(usize, String)>,
    pub structured: JsonScan,
}

pub async fn process_output_response<Guardrail>(
    result: &mut CallToolResult,
    guardrail: &Guardrail,
) -> Result<(), TranslationError<Guardrail::Error>>
where
    Guardrail: ApplyGuardrail + Sync,
{
    let scan = OutputScan::of(result)?;
    let texts = scan.texts();
    if texts.is_empty() {
        tracing::debug!("MCP guardrail: tool result has no scannable text");
        return Ok(());
    }
    let scanned = texts.len();
    let inputs = GuardrailInputs {
        tools: Vec::new(),
        texts,
    };
    let returned = guardrail
        .apply_guardrail(inputs, GuardrailInputType::Response)
        .await
        .map_err(TranslationError::Guardrail)?;
    let Some(masked) = masked_texts(returned, scanned, ScanTarget::ToolResult)? else {
        return Ok(());
    };
    mask_result(&scan, result, masked)?;
    Ok(())
}

impl OutputScan {
    pub fn of(result: &CallToolResult) -> Result<Self, ContentBlocked> {
        let mut text_blocks = Vec::new();
        for (index, item) in result.content.iter().enumerate() {
            if let Some(text) = &item.text {
                text_blocks.push((index, text.clone()));
            }
        }
        let structured = match &result.structured_content {
            Some(value) => scan_json(value, NestedPayload::StructuredContent)?,
            None => JsonScan::default(),
        };
        Ok(Self {
            text_blocks,
            structured,
        })
    }

    #[must_use]
    pub fn texts(&self) -> Vec<String> {
        let mut texts: Vec<String> = self
            .text_blocks
            .iter()
            .map(|(_, text)| text.clone())
            .collect();
        texts.extend(self.structured.leaves.iter().map(|leaf| leaf.text.clone()));
        texts.extend(self.structured.labels.iter().cloned());
        texts
    }
}
