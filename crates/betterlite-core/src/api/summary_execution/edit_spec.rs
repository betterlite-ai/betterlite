use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};

use crate::api::messages_context_clear::constants::COMPACT_EDIT_TYPE;
use crate::api::messages_context_clear::error::ContextManagementError;
use crate::api::messages_context_clear::result::AppliedEdit;
use crate::api::summary_history::trigger::resolve_trigger_tokens;

// every key compact_20260112 reads from the client's edit spec, kept as the client sent it
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CompactEditSpec {
    #[serde(rename = "type", default)]
    pub edit_type: Option<Json>,
    #[serde(default)]
    pub trigger: Option<Json>,
    // pause_after_compaction is tested for truthiness, so null and false both leave it off
    #[serde(default)]
    pub pause_after_compaction: Option<Json>,
    #[serde(default)]
    pub instructions: Option<Json>,
}

// validation runs first, and its only abort is the trigger error; the pause knob only adds a warning
pub fn validate(edit_spec: &CompactEditSpec) -> Result<(u64, AppliedEdit), ContextManagementError> {
    let resolved = resolve_trigger_tokens(edit_spec.trigger.as_ref())?;
    tracing::info!(
        "compact_20260112: request has compaction trigger (input_tokens threshold={})",
        resolved.tokens
    );
    let mut warnings = resolved.warnings;
    if edit_spec
        .pause_after_compaction
        .as_ref()
        .is_some_and(Json::is_truthy)
    {
        warnings.push("pause_after_compaction_ignored".to_owned());
    }
    let applied = AppliedEdit {
        edit_type: Some(COMPACT_EDIT_TYPE.to_owned()),
        cleared_input_tokens: None,
        cleared_tool_uses: None,
        cleared_thinking_turns: None,
        summary_input_tokens: None,
        summary_output_tokens: None,
        error: None,
        warnings: (!warnings.is_empty()).then_some(warnings),
    };
    Ok((resolved.tokens, applied))
}
