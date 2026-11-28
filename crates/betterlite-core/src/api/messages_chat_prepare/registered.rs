use async_trait::async_trait;

use crate::api::messages_chat_prepare::prepare::{DispatchFailure, PolyfillDispatch};
use crate::api::messages_compaction::dispatch::{DispatchRequest, apply_context_management};
use crate::api::messages_compaction::editor::{EditorFailure, RegisteredEditors};
use crate::api::messages_compaction::error::DispatchError;
use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::summary_execution::error::SummaryExecutionError;
use crate::api::summary_history::count::HistoryTokenCounter;
use crate::api::summary_model_access::checks::SummaryModelChecks;
use crate::api::summary_model_access::repositories::SummaryAccessRepositories;
use crate::api::summary_model_limits::budget::SummaryModelBudgets;
use crate::api::summary_model_limits::rate::ProxyHooks;

// the real editors behind the wrapper: a trigger error keeps its status and text, everything else is best-effort
#[async_trait]
impl<Checks, Repositories, Budgets, Hooks, HistoryCounter> PolyfillDispatch
    for RegisteredEditors<'_, Checks, Repositories, Budgets, Hooks, HistoryCounter>
where
    Checks: SummaryModelChecks,
    Repositories: SummaryAccessRepositories,
    Budgets: SummaryModelBudgets,
    Hooks: ProxyHooks,
    HistoryCounter: HistoryTokenCounter + Sync,
{
    async fn dispatch(
        &self,
        request: DispatchRequest<'_>,
    ) -> Result<Option<PolyfillResult>, DispatchFailure> {
        match apply_context_management(request, self).await {
            Ok(result) => Ok(Some(result)),
            Err(DispatchError::Editor(EditorFailure::Compact(SummaryExecutionError::Trigger(
                trigger,
            )))) => Err(DispatchFailure::Context(trigger)),
            Err(error) => Err(DispatchFailure::Other(error.to_string())),
        }
    }
}
