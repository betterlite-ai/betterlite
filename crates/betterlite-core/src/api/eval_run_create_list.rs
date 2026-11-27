pub mod args;
pub mod error;
pub mod providers;

mod fields;
mod prepare;
mod tests;

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_openai::evals::types::{ListRunsResponse, Run};
use betterlite_redact::secret::Redactor;

use self::args::{CreateRunArgs, ListRunsArgs, local_vars_create, local_vars_list};
use self::error::{EvalRunCallError, EvalRunFailure};
use self::prepare::create::prepare_create;
use self::prepare::list::prepare_list;
use self::providers::{
    CREATE_FLAG, EvalRunProviders, LIST_FLAG, PendingCreateRun, PendingListRuns, RunMode,
    RunTransport,
};
use crate::api::eval_http::exchange::EvalLogging;

// LiteLLM's provider fallback when the caller names none
pub(crate) const DEFAULT_PROVIDER: &str = "openai";
// create_run's own default, not the standard request timeout: httpx.Timeout(timeout=600.0, connect=5.0)
pub(crate) const CREATE_TIMEOUT: Duration = Duration::from_secs(600);

// what the blocking entry hands back: the object, or the handler future async mode asked it to return
pub enum CreateRunWork<'call> {
    Ready(Box<Run>),
    Pending(PendingCreateRun<'call>),
}

pub enum ListRunsWork<'call> {
    Ready(Box<ListRunsResponse>),
    Pending(PendingListRuns<'call>),
}

// the injected collaborators: B5360's selector, B9371's handlers, B185/B186's redactor and the request timeout constant
pub struct EvalRunCreateList<Providers, Transport> {
    pub providers: Providers,
    pub transport: Transport,
    pub redactor: Redactor,
    pub request_timeout: Duration,
}

enum Polled<Output> {
    Ready(Output),
    StillWaiting,
}

// the four entries differ only in the request, the work type, the prepare step, the transport method and the local_vars
macro_rules! eval_run_operations {
    (sync $name:ident, $request:ident, $work:ident, $prepare:path, $transport:ident, $local_vars:path) => {
        pub fn $name<'call>(
            &'call self,
            request: $request<'_>,
            log: EvalLogging<'call, '_>,
        ) -> Result<$work<'call>, EvalRunFailure> {
            let provider = request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
            let call = match $prepare(self, request, provider, log.state) {
                Ok(call) => call,
                Err(error) => {
                    return Err(EvalRunFailure::of(
                        error,
                        provider,
                        &self.redactor,
                        $local_vars(&request),
                    ));
                }
            };
            let mode = call.mode;
            let pending = self.transport.$transport(log, call);
            if mode == RunMode::Async {
                return Ok($work::Pending(pending));
            }
            match poll_once(pending) {
                Polled::Ready(Ok(answer)) => Ok($work::Ready(Box::new(answer))),
                Polled::Ready(Err(error)) => Err(EvalRunFailure::of(
                    error,
                    provider,
                    &self.redactor,
                    $local_vars(&request),
                )),
                Polled::StillWaiting => Err(EvalRunFailure::of(
                    EvalRunCallError::Suspended,
                    provider,
                    &self.redactor,
                    $local_vars(&request),
                )),
            }
        }
    };
    (async $name:ident, $inner:ident, $request:ident, $work:ident, $flag:ident, $reply:ty, $local_vars:path) => {
        pub async fn $name<'call>(
            &'call self,
            request: $request<'_>,
            log: EvalLogging<'call, '_>,
        ) -> Result<$reply, EvalRunFailure> {
            let provider = request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
            let mut kwargs = request.kwargs.clone();
            kwargs.insert($flag.to_owned(), Json::Bool(true));
            let flagged = $request {
                kwargs: &kwargs,
                ..request
            };
            match self.$inner(flagged, log) {
                Ok($work::Ready(answer)) => Ok(*answer),
                Ok($work::Pending(pending)) => pending.await.map_err(|error| {
                    EvalRunFailure::of(error, provider, &self.redactor, $local_vars(&flagged))
                }),
                Err(failure) => Err(failure),
            }
        }
    };
}

impl<Providers, Transport> EvalRunCreateList<Providers, Transport>
where
    Providers: EvalRunProviders,
    Transport: RunTransport,
{
    // LiteLLM's create_run: the provider, the logging update, the transform, then the handler the flag picks
    eval_run_operations!(
        sync create_run,
        CreateRunArgs,
        CreateRunWork,
        prepare_create,
        create_run,
        local_vars_create
    );

    // LiteLLM's acreate_run: the flag in kwargs, then the sync call's own work with its coroutine awaited
    eval_run_operations!(
        async acreate_run,
        create_run,
        CreateRunArgs,
        CreateRunWork,
        CREATE_FLAG,
        Run,
        local_vars_create
    );

    // LiteLLM's list_runs: the four pagination values, extra_query's overrides, then the GET handler
    eval_run_operations!(
        sync list_runs,
        ListRunsArgs,
        ListRunsWork,
        prepare_list,
        list_runs,
        local_vars_list
    );

    // LiteLLM's alist_runs: the flag in kwargs, then list_runs with its coroutine awaited
    eval_run_operations!(
        async alist_runs,
        list_runs,
        ListRunsArgs,
        ListRunsWork,
        LIST_FLAG,
        ListRunsResponse,
        local_vars_list
    );
}

// the blocking path's single poll: a sync handler answers at once, so only a broken one leaves this waiting
fn poll_once<Running>(future: Running) -> Polled<Running::Output>
where
    Running: Future,
{
    let mut pinned = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match pinned.as_mut().poll(&mut context) {
        Poll::Ready(outcome) => Polled::Ready(outcome),
        Poll::Pending => Polled::StillWaiting,
    }
}
