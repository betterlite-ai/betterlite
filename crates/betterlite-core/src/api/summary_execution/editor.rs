use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::summary_model_access::Settings;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::api::messages_context_clear::result::PolyfillResult;
use crate::api::summary_execution::completion::{
    SummaryCall, SummaryCallFailure, SummaryCompletion,
};
use crate::api::summary_execution::draft::{PhaseA, phase_a, slice_only, tail};
use crate::api::summary_execution::edit_spec::{CompactEditSpec, validate};
use crate::api::summary_execution::encoding::encode;
use crate::api::summary_execution::error::SummaryExecutionError;
use crate::api::summary_execution::extract::CompletionAnswer;
use crate::api::summary_execution::policy::policy_denial;
use crate::api::summary_execution::summarize::{SummaryTask, summarize};
use crate::api::summary_history::count::{
    HistoryTokenCounter, TokenComponents, count_effective_tokens,
};
use crate::api::summary_history::system::SystemPrompt;
use crate::api::summary_history::translate::AdapterTranslator;
use crate::api::summary_model_access::access::ProxyGlobals;
use crate::api::summary_model_access::checks::SummaryModelChecks;
use crate::api::summary_model_access::repositories::SummaryAccessRepositories;
use crate::api::summary_model_access::settings::read_summary_settings;
use crate::api::summary_model_limits::budget::SummaryModelBudgets;
use crate::api::summary_model_limits::rate::ProxyHooks;

// apply_compact_20260112's arguments: the caller's history plus the request-wide system and metadata
pub struct EditorRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Json],
    pub tools: Option<&'request [Json]>,
    pub system: Option<&'request SystemPrompt>,
    pub edit_spec: &'request CompactEditSpec,
    pub litellm_metadata: Option<&'request Object>,
    pub principal: Option<&'request UserAPIKeyAuth>,
}

// everything the editor calls that lives outside it: B969's settings and access, B970's limits, B971's counter, the two completions
pub struct EditorServices<'services, Checks, Repositories, Budgets, Hooks, Counter> {
    pub settings: Option<&'services Settings>,
    pub proxy: Option<&'services ProxyGlobals<'services, Checks, Repositories>>,
    pub budgets: Option<&'services Budgets>,
    pub hooks: Option<&'services Hooks>,
    pub counter: &'services Counter,
    pub router: Option<&'services dyn SummaryCompletion>,
    pub direct: &'services dyn SummaryCompletion,
}

impl<Checks, Repositories, Budgets, Hooks, Counter>
    EditorServices<'_, Checks, Repositories, Budgets, Hooks, Counter>
{
    // compact.py prefers llm_router.acompletion and falls back to litellm.acompletion
    async fn complete(
        &self,
        call: SummaryCall<'_>,
    ) -> Result<CompletionAnswer, SummaryCallFailure> {
        match self.router {
            Some(router) => router.complete(call).await,
            None => self.direct.complete(call).await,
        }
    }
}

pub async fn apply_compact_20260112<Checks, Repositories, Budgets, Hooks, Counter>(
    request: EditorRequest<'_>,
    services: &EditorServices<'_, Checks, Repositories, Budgets, Hooks, Counter>,
) -> Result<PolyfillResult, SummaryExecutionError<Hooks::Error>>
where
    Checks: SummaryModelChecks,
    Repositories: SummaryAccessRepositories,
    Budgets: SummaryModelBudgets,
    Hooks: ProxyHooks,
    Counter: HistoryTokenCounter + Sync,
{
    let (trigger, applied) = validate(request.edit_spec)?;
    let phase = phase_a(request.messages, request.system);
    let (summary_model, max_tokens) = read_summary_settings(services.settings);
    let Some(summary_model) = summary_model else {
        let mut applied = applied;
        applied.error = Some("summary_model_not_configured".to_owned());
        return encode(slice_only(&phase, applied));
    };
    let current = measure(&request, &phase, services.counter).await;
    tracing::debug!(
        "compact_20260112: current_tokens={} trigger={}",
        current,
        trigger
    );
    if current <= trigger {
        return encode(slice_only(&phase, applied));
    }
    let denial = policy_denial(
        request.principal,
        services.proxy,
        services.budgets,
        services.hooks,
        &summary_model,
    )
    .await?;
    if let Some(denial) = denial {
        tracing::warn!("{}", denial.warning(&summary_model));
        let mut applied = applied;
        applied.error = Some(denial.code().to_owned());
        return encode(tail(&phase, applied));
    }
    let task = SummaryTask {
        request: &request,
        phase: &phase,
        summary_model: &summary_model,
        max_tokens,
        applied,
    };
    let draft = summarize(task, services).await?;
    encode(draft)
}

// a failed count is not fatal: LiteLLM warns and assumes the request is under the threshold
async fn measure<Counter>(request: &EditorRequest<'_>, phase: &PhaseA, counter: &Counter) -> usize
where
    Counter: HistoryTokenCounter + Sync,
{
    let components = TokenComponents {
        model: request.model,
        messages: &phase.effective,
        compaction_block: None,
        tools: request.tools,
        system: phase.system.as_ref(),
    };
    let translator = AdapterTranslator;
    match count_effective_tokens(components, &translator, counter).await {
        Ok(tokens) => tokens,
        Err(error) => {
            tracing::warn!(
                "compact_20260112: token_counter failed; assuming under threshold: {}",
                error
            );
            0
        }
    }
}
