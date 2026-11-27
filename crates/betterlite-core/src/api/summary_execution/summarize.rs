use crate::api::messages_context_clear::constants::SUMMARY_TIMEOUT_SECONDS;
use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, CompactionKind, UsageIteration, UsageKind,
};
use crate::api::summary_execution::attribution::{PrincipalRegion, end_user};
use crate::api::summary_execution::completion::SummaryCall;
use crate::api::summary_execution::draft::{Draft, PhaseA, forward};
use crate::api::summary_execution::editor::{EditorRequest, EditorServices};
use crate::api::summary_execution::error::SummaryExecutionError;
use crate::api::summary_execution::extract::{
    CompletionAnswer, extract_response_text, extract_usage,
};
use crate::api::summary_history::last_question::select_last_user_question;
use crate::api::summary_history::metadata::propagate_metadata;
use crate::api::summary_history::prompt::{build_summary_prompt, extract_summary_text};
use crate::api::summary_history::summary_messages::build_summary_messages;
use crate::api::summary_history::system::{SystemPrompt, augment_system_with_summary};
use crate::api::summary_history::translate::AdapterTranslator;
use crate::api::summary_model_limits::rate::ProxyHooks;

// one above-threshold summary: the request's own history and system, the chosen model, and the applied edit so far
pub struct SummaryTask<'task, 'request> {
    pub request: &'task EditorRequest<'request>,
    pub phase: &'task PhaseA,
    pub summary_model: &'task str,
    pub max_tokens: u64,
    pub applied: AppliedEdit,
}

pub async fn summarize<Checks, Repositories, Budgets, Hooks, Counter>(
    task: SummaryTask<'_, '_>,
    services: &EditorServices<'_, Checks, Repositories, Budgets, Hooks, Counter>,
) -> Result<Draft, SummaryExecutionError<Hooks::Error>>
where
    Hooks: ProxyHooks,
{
    let prompt = build_summary_prompt(
        task.request.edit_spec.instructions.as_ref(),
        task.request.tools,
    );
    let translator = AdapterTranslator;
    let messages = build_summary_messages(
        &task.phase.effective,
        &prompt,
        task.phase.system.as_ref(),
        &translator,
    )
    .await;
    let metadata = propagate_metadata(task.request.litellm_metadata);
    let region = match task.request.principal {
        Some(principal) => {
            let encoded =
                serde_json::to_value(principal).map_err(SummaryExecutionError::Principal)?;
            let fields: PrincipalRegion =
                serde_json::from_value(encoded).map_err(SummaryExecutionError::Principal)?;
            fields.allowed_model_region
        }
        None => None,
    };
    let call = SummaryCall {
        model: task.summary_model,
        messages: &messages,
        max_tokens: task.max_tokens,
        timeout_seconds: SUMMARY_TIMEOUT_SECONDS,
        metadata: &metadata,
        user: end_user(task.request.litellm_metadata),
        allowed_model_region: region.as_deref(),
    };
    match services.complete(call).await {
        Ok(answer) => finish(task, &answer),
        Err(failure) => {
            tracing::warn!("compact_20260112: summary call failed: {}", failure);
            let mut applied = task.applied;
            applied.error = Some("summary_call_failed".to_owned());
            Ok(forward(
                task.phase.downstream.clone(),
                task.phase.system.clone(),
                applied,
            ))
        }
    }
}

// a response without a usable summary tag is nonfatal and logs no call-failure warning
fn finish<HookError>(
    task: SummaryTask<'_, '_>,
    answer: &CompletionAnswer,
) -> Result<Draft, SummaryExecutionError<HookError>> {
    let raw = extract_response_text(answer)?;
    let summary = extract_summary_text(raw.as_deref())?;
    let Some(summary) = summary else {
        let mut applied = task.applied;
        applied.error = Some("summary_extraction_failed".to_owned());
        return Ok(forward(
            task.phase.downstream.clone(),
            task.phase.system.clone(),
            applied,
        ));
    };
    let usage = extract_usage(answer)?;
    let mut applied = task.applied;
    applied.summary_input_tokens = Some(usize::try_from(usage.input_tokens).unwrap_or(usize::MAX));
    applied.summary_output_tokens =
        Some(usize::try_from(usage.output_tokens).unwrap_or(usize::MAX));
    let system = augment_system_with_summary(task.request.system, &summary);
    tracing::info!(
        "compact_20260112: compaction summary added to main call system prefix ({} chars)",
        summary.chars().count()
    );
    Ok(Draft {
        messages: vec![select_last_user_question(&task.phase.effective)],
        system: Some(system),
        applied,
        compaction_block: Some(CompactionBlock {
            kind: CompactionKind::Compaction,
            content: Some(summary),
            signature: None,
        }),
        iterations_usage: Some(vec![UsageIteration {
            kind: UsageKind::Compaction,
            input_tokens: Some(usage.input_tokens),
            output_tokens: Some(usage.output_tokens),
        }]),
    })
}
