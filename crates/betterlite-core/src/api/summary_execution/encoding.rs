use betterlite_models::decisions::Json as DecisionJson;

use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::summary_execution::draft::Draft;
use crate::api::summary_execution::error::SummaryExecutionError;

// the editor's own history and system types become the caller's result type here, never inside the steps
pub fn encode<HookError>(draft: Draft) -> Result<PolyfillResult, SummaryExecutionError<HookError>> {
    let messages: Vec<DecisionJson> = serde_json::to_value(&draft.messages)
        .and_then(serde_json::from_value)
        .map_err(SummaryExecutionError::Encode)?;
    let system: Option<DecisionJson> = match draft.system.as_ref() {
        Some(prompt) => {
            let value = serde_json::to_value(prompt).map_err(SummaryExecutionError::Encode)?;
            Some(serde_json::from_value(value).map_err(SummaryExecutionError::Encode)?)
        }
        None => None,
    };
    Ok(PolyfillResult {
        messages,
        system,
        applied_edits: vec![draft.applied],
        compaction_block: draft.compaction_block,
        iterations_usage: draft.iterations_usage,
    })
}
