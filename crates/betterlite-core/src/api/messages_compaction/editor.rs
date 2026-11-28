use std::error::Error;
use std::sync::Arc;

use betterlite_config::sections::summary_model_access::Settings;
use betterlite_prompts::arguments::json::Json as PromptJson;
use thiserror::Error;
use tokio::task::{JoinError, JoinSet};

use crate::api::messages_compaction::convert::{SystemShapeError, prompt_json, system_prompt};
use crate::api::messages_compaction::dispatch::{
    ClearCall, CompactCall, ContextEditor, EditCall, EditorOutcome,
};
use crate::api::messages_context_clear::counter::TokenCounter;
use crate::api::messages_context_clear::editor::{
    ClearToolUsesOutcome, EditorRequest as ClearEditorRequest, apply_clear_tool_uses_20250919,
};
use crate::api::messages_context_clear::error::ClearToolUsesError;
use crate::api::messages_context_clear::spec::ClearToolUsesEditSpec;
use crate::api::summary_execution::completion::SummaryCompletion;
use crate::api::summary_execution::edit_spec::CompactEditSpec;
use crate::api::summary_execution::editor::{
    EditorRequest as CompactEditorRequest, EditorServices, apply_compact_20260112,
};
use crate::api::summary_execution::error::SummaryExecutionError;
use crate::api::summary_history::count::HistoryTokenCounter;
use crate::api::summary_model_access::access::ProxyGlobals;
use crate::api::summary_model_access::checks::SummaryModelChecks;
use crate::api::summary_model_access::repositories::SummaryAccessRepositories;
use crate::api::summary_model_limits::budget::SummaryModelBudgets;
use crate::api::summary_model_limits::rate::ProxyHooks;

// what a registered editor can refuse: the client's spec, the system shape, the editor itself or its blocking task
#[derive(Debug, Error)]
pub enum EditorFailure<HookError>
where
    HookError: Error + 'static,
{
    #[error(transparent)]
    System(#[from] SystemShapeError),
    #[error("the context edit's spec does not parse")]
    Spec(#[source] serde_json::Error),
    #[error(transparent)]
    Clear(#[from] ClearToolUsesError),
    #[error(transparent)]
    Compact(#[from] SummaryExecutionError<HookError>),
    #[error("the clear editor's blocking task failed")]
    Task(#[source] JoinError),
    #[error("the clear editor's blocking task ended without a result")]
    MissingResult,
}

// the two registry entries: B968's sync editor off the executor and B2432's async editor awaited in place
pub struct RegisteredEditors<'services, Checks, Repositories, Budgets, Hooks, HistoryCounter> {
    pub settings: Option<&'services Settings>,
    pub proxy: Option<&'services ProxyGlobals<'services, Checks, Repositories>>,
    pub budgets: Option<&'services Budgets>,
    pub hooks: Option<&'services Hooks>,
    pub history_counter: &'services HistoryCounter,
    pub clear_counter: Arc<dyn TokenCounter + Send + Sync>,
    pub direct: &'services dyn SummaryCompletion,
}

impl<Checks, Repositories, Budgets, Hooks, HistoryCounter>
    RegisteredEditors<'_, Checks, Repositories, Budgets, Hooks, HistoryCounter>
where
    Checks: SummaryModelChecks,
    Repositories: SummaryAccessRepositories,
    Budgets: SummaryModelBudgets,
    Hooks: ProxyHooks,
    HistoryCounter: HistoryTokenCounter + Sync,
{
    // B968's editor is sync: the blocking pool runs it and the dispatcher awaits its JoinSet entry
    async fn clear_outcome(
        &self,
        clear: ClearCall,
    ) -> Result<EditorOutcome, EditorFailure<Hooks::Error>> {
        // both specs are read out of the edit the client sent, as LiteLLM hands the dict to each editor
        let edit_spec: ClearToolUsesEditSpec = serde_json::from_value(
            serde_json::to_value(&clear.edit_spec).map_err(EditorFailure::Spec)?,
        )
        .map_err(EditorFailure::Spec)?;
        let ClearCall {
            model,
            messages,
            tools,
            system,
            ..
        } = clear;
        let counter = Arc::clone(&self.clear_counter);
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || {
            apply_clear_tool_uses_20250919(
                ClearEditorRequest {
                    model: &model,
                    messages,
                    tools: tools.as_deref(),
                    system: system.as_ref(),
                    edit_spec: &edit_spec,
                },
                counter.as_ref(),
            )
        });
        let outcome: ClearToolUsesOutcome = match tasks.join_next().await {
            Some(Ok(result)) => result.map_err(EditorFailure::Clear)?,
            Some(Err(source)) => return Err(EditorFailure::Task(source)),
            None => return Err(EditorFailure::MissingResult),
        };
        Ok(EditorOutcome::Legacy {
            messages: outcome.messages,
            applied: outcome.applied,
        })
    }

    // B2432's editor is async: the summary call is awaited in place, not spawned
    async fn compact_outcome(
        &self,
        compact: CompactCall<'_>,
    ) -> Result<EditorOutcome, EditorFailure<Hooks::Error>> {
        let edit_spec: CompactEditSpec = serde_json::from_value(
            serde_json::to_value(&compact.edit_spec).map_err(EditorFailure::Spec)?,
        )
        .map_err(EditorFailure::Spec)?;
        let messages: Vec<PromptJson> = compact.messages.iter().map(prompt_json).collect();
        let tools: Option<Vec<PromptJson>> = compact
            .tools
            .as_deref()
            .map(|tools| tools.iter().map(prompt_json).collect());
        let system = system_prompt(compact.system.as_ref())?;
        let services = EditorServices {
            settings: self.settings,
            proxy: self.proxy,
            budgets: self.budgets,
            hooks: self.hooks,
            counter: self.history_counter,
            router: compact.llm_router,
            direct: self.direct,
        };
        let request = CompactEditorRequest {
            model: compact.model,
            messages: &messages,
            tools: tools.as_deref(),
            system: system.as_ref(),
            edit_spec: &edit_spec,
            litellm_metadata: compact.litellm_metadata,
            principal: compact.user_api_key_auth,
        };
        let result = apply_compact_20260112(request, &services).await?;
        Ok(EditorOutcome::Polyfilled(result))
    }
}

impl<Checks, Repositories, Budgets, Hooks, HistoryCounter> ContextEditor
    for RegisteredEditors<'_, Checks, Repositories, Budgets, Hooks, HistoryCounter>
where
    Checks: SummaryModelChecks,
    Repositories: SummaryAccessRepositories,
    Budgets: SummaryModelBudgets,
    Hooks: ProxyHooks,
    HistoryCounter: HistoryTokenCounter + Sync,
{
    type Failure = EditorFailure<Hooks::Error>;

    async fn edit(&self, call: EditCall<'_>) -> Result<EditorOutcome, Self::Failure> {
        match call {
            EditCall::Clear(clear) => self.clear_outcome(clear).await,
            EditCall::Compact(compact) => self.compact_outcome(compact).await,
        }
    }
}
