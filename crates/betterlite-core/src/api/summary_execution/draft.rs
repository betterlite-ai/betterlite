use betterlite_prompts::arguments::json::Json;

use crate::api::messages_context_clear::result::{AppliedEdit, CompactionBlock, UsageIteration};
use crate::api::summary_history::boundary::{
    slice_around_compaction_block, strip_compaction_blocks,
};
use crate::api::summary_history::last_question::select_last_user_question;
use crate::api::summary_history::system::{SystemPrompt, augment_system_with_summary};

// Phase A's history: the slice from the latest compaction block on, its stripped form, and the augmented system
pub struct PhaseA {
    pub effective: Vec<Json>,
    pub downstream: Vec<Json>,
    pub system: Option<SystemPrompt>,
}

// what the editor returns before it is encoded onto the caller's result type
pub struct Draft {
    pub messages: Vec<Json>,
    pub system: Option<SystemPrompt>,
    pub applied: AppliedEdit,
    pub compaction_block: Option<CompactionBlock>,
    pub iterations_usage: Option<Vec<UsageIteration>>,
}

// splitting around the latest compaction block runs before any setting is read, so blocks never leak downstream
pub fn phase_a(messages: &[Json], system: Option<&SystemPrompt>) -> PhaseA {
    let (effective, block) = match slice_around_compaction_block(messages) {
        Some(slice) => (slice.messages, Some(slice.block)),
        None => (messages.to_vec(), None),
    };
    let prior = block
        .as_ref()
        .and_then(|fields| fields.get("content"))
        .and_then(Json::as_str)
        .filter(|text| !text.is_empty());
    let augmented = prior.map_or_else(
        || system.cloned(),
        |summary| {
            tracing::info!(
                "compact_20260112: compaction summary added to main call system prefix ({} chars)",
                summary.chars().count()
            );
            Some(augment_system_with_summary(system, summary))
        },
    );
    let downstream = strip_compaction_blocks(&effective);
    PhaseA {
        effective,
        downstream,
        system: augmented,
    }
}

// the slice-only exits keep the stripped tail and fall back to the last question only when it is empty
#[must_use]
pub fn slice_only(phase: &PhaseA, applied: AppliedEdit) -> Draft {
    let messages = if phase.downstream.is_empty() {
        vec![select_last_user_question(&phase.effective)]
    } else {
        phase.downstream.clone()
    };
    forward(messages, phase.system.clone(), applied)
}

// the slice-only and skipped-summary exits fill the same five fields; only how they pick the messages differs
pub(super) const fn forward(
    messages: Vec<Json>,
    system: Option<SystemPrompt>,
    applied: AppliedEdit,
) -> Draft {
    Draft {
        messages,
        system,
        applied,
        compaction_block: None,
        iterations_usage: None,
    }
}
