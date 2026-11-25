use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;

use crate::api::responses_agentic_execution::executor::ContinuationExecutor;
use crate::api::responses_create::bridge::Created;
use crate::api::responses_create::call::ResponsesCreateCall;
use crate::api::responses_create::deps::{CreateConfigFactory, ResponsesCreateDeps};
use crate::api::responses_create::error::CreateError;
use crate::api::responses_create::execute::create;
use crate::api::responses_create_http::service::{ClientSource, LogContext};
use crate::api::responses_hook_policy::callbacks::DeploymentHookDispatcher;
use crate::api::responses_provider_selection::prompting::{
    HookRequest, MergeAnswer, MergeRequest, PromptManagerError,
};
use crate::api::responses_provider_selection::provider_config::ConfigLookup;
use crate::api::responses_provider_selection::resolution::ModelResolver;

// LiteLLMLoggingObj.should_run_prompt_management_hooks and its async merge hook, behind a port the per-call dispatch fills
#[async_trait]
pub trait AsyncPromptPort: Send + Sync {
    fn runs_prompt_hooks(&self, request: &HookRequest<'_>) -> bool;

    async fn async_merge_prompt(
        &self,
        request: &MergeRequest<'_>,
    ) -> Result<MergeAnswer, PromptManagerError>;
}

// the ports aresponses reads itself: provider detection, the bridge decision and the async prompt hook
pub struct AsyncCreateDeps<'deps> {
    pub resolver: &'deps (dyn ModelResolver + Send + Sync),
    pub lookup: &'deps (dyn ConfigLookup + Send + Sync),
    pub prompts: Option<&'deps dyn AsyncPromptPort>,
}

// what the sync call left behind: a response (or None), or the coroutine aresponses awaits exactly once
pub type PendingCreate<'call, Context> =
    Pin<Box<dyn Future<Output = Result<Option<Created<Context>>, CreateError>> + Send + 'call>>;

pub enum CreateReturn<'call, Context> {
    Completed(Option<Created<Context>>),
    Pending(PendingCreate<'call, Context>),
}

// the synchronous responses call aresponses hands its converted arguments to
pub trait SyncHandoff<Context>: Send + Sync {
    fn handoff<'call>(
        &'call self,
        call: &'call ResponsesCreateCall,
        context: Context,
        log: &'call mut LogContext<'_, '_>,
    ) -> CreateReturn<'call, Context>
    where
        Context: 'call;
}

// B1078's create behind the handoff, so the production entry awaits the whole sync execution once
pub struct CreateExecution<'deps, S, F, D: ?Sized, E: ?Sized, Context>
where
    S: ClientSource,
    Context: Send,
{
    pub deps: &'deps ResponsesCreateDeps<'deps, S, F, D, E, Context>,
}

impl<S, F, D, E, Context> SyncHandoff<Context> for CreateExecution<'_, S, F, D, E, Context>
where
    S: ClientSource,
    F: CreateConfigFactory,
    D: DeploymentHookDispatcher + ?Sized,
    E: ContinuationExecutor + ?Sized,
    Context: Send,
{
    fn handoff<'call>(
        &'call self,
        call: &'call ResponsesCreateCall,
        context: Context,
        log: &'call mut LogContext<'_, '_>,
    ) -> CreateReturn<'call, Context>
    where
        Context: 'call,
    {
        let deps = self.deps;
        CreateReturn::Pending(Box::pin(async move {
            create(deps, call, context, log).await.map(Some)
        }))
    }
}
