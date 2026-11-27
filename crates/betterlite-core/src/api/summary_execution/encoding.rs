use betterlite_models::decisions::Json as DecisionJson;

use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::summary_execution::draft::Draft;
use crate::api::summary_execution::error::SummaryExecutionError;
use crate::api::summary_execution::roundtrip::convert;

// the editor's own history and system types become the caller's result type here, never inside the steps
pub fn encode<HookError>(draft: Draft) -> Result<PolyfillResult, SummaryExecutionError<HookError>> {
    let messages: Vec<DecisionJson> =
        convert(&draft.messages).map_err(SummaryExecutionError::Encode)?;
    let system: Option<DecisionJson> = draft
        .system
        .as_ref()
        .map(convert)
        .transpose()
        .map_err(SummaryExecutionError::Encode)?;
    Ok(PolyfillResult {
        messages,
        system,
        applied_edits: vec![draft.applied],
        compaction_block: draft.compaction_block,
        iterations_usage: draft.iterations_usage,
    })
}
