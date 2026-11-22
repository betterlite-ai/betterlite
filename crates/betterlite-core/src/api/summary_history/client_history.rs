use betterlite_prompts::arguments::json::Json;

use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::summary_history::boundary::{
    slice_around_compaction_block, strip_compaction_blocks,
};
use crate::api::summary_history::error::SummaryHistoryError;
use crate::api::summary_history::last_question::select_last_user_question;
use crate::api::summary_history::system::{SystemPrompt, augment_system_with_summary};

// a client-side compaction block is honored without a context_management edit, keeping the turns after it
pub fn client_compaction_history(
    messages: &[Json],
    system: Option<&SystemPrompt>,
) -> Result<Option<PolyfillResult>, SummaryHistoryError> {
    let Some(slice) = slice_around_compaction_block(messages) else {
        return Ok(None);
    };
    tracing::info!(
        "compact_20260112: client compaction block in message history; applying slice-only forwarding (no context_management edit)"
    );
    let augmented = match slice.block.get("content") {
        Some(Json::Text(summary)) if !summary.is_empty() => {
            let prefixed = augment_system_with_summary(system, summary);
            tracing::info!(
                "compact_20260112: compaction summary added to main call system prefix ({} chars)",
                summary.chars().count()
            );
            Some(prefixed)
        }
        Some(_) | None => system.cloned(),
    };
    let mut downstream = strip_compaction_blocks(&slice.messages);
    if downstream.is_empty() {
        downstream.push(select_last_user_question(&slice.messages));
    }
    Ok(Some(PolyfillResult {
        messages: serde_json::to_value(&downstream).and_then(serde_json::from_value)?,
        system: augmented
            .map(|prompt| serde_json::to_value(&prompt).and_then(serde_json::from_value))
            .transpose()?,
        applied_edits: Vec::new(),
        compaction_block: None,
        iterations_usage: None,
    }))
}
