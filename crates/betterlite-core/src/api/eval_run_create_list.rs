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

use self::args::{CreateRunArgs, ListRunsArgs};
use self::error::{EvalRunCallError, EvalRunFailure, create_failure, list_failure};
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

impl<Providers, Transport> EvalRunCreateList<Providers, Transport>
where
    Providers: EvalRunProviders,
    Transport: RunTransport,
{
    // LiteLLM's create_run: the provider, the logging update, the transform, then the handler the flag picks
    pub fn create_run<'call>(
        &'call self,
        request: CreateRunArgs<'_>,
        log: EvalLogging<'call, '_>,
    ) -> Result<CreateRunWork<'call>, EvalRunFailure> {
        let provider = request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
        let call = match self.prepare_create(request, provider, log.state) {
            Ok(call) => call,
            Err(error) => return Err(create_failure(error, provider, &request, &self.redactor)),
        };
        let mode = call.mode;
        let pending = self.transport.create_run(log, call);
        if mode == RunMode::Async {
            return Ok(CreateRunWork::Pending(pending));
        }
        match poll_once(pending) {
            Polled::Ready(Ok(run)) => Ok(CreateRunWork::Ready(Box::new(run))),
            Polled::Ready(Err(error)) => {
                Err(create_failure(error, provider, &request, &self.redactor))
            }
            Polled::StillWaiting => Err(create_failure(
                EvalRunCallError::Suspended,
                provider,
                &request,
                &self.redactor,
            )),
        }
    }

    // LiteLLM's acreate_run: the flag in kwargs, then the sync call's own work with its coroutine awaited
    pub async fn acreate_run<'call>(
        &'call self,
        request: CreateRunArgs<'_>,
        log: EvalLogging<'call, '_>,
    ) -> Result<Run, EvalRunFailure> {
        let provider = request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
        let mut kwargs = request.kwargs.clone();
        kwargs.insert(CREATE_FLAG.to_owned(), Json::Bool(true));
        let flagged = CreateRunArgs {
            kwargs: &kwargs,
            ..request
        };
        match self.create_run(flagged, log) {
            Ok(CreateRunWork::Ready(run)) => Ok(*run),
            Ok(CreateRunWork::Pending(pending)) => pending
                .await
                .map_err(|error| create_failure(error, provider, &flagged, &self.redactor)),
            Err(failure) => Err(failure),
        }
    }

    // LiteLLM's list_runs: the four pagination values, extra_query's overrides, then the GET handler
    pub fn list_runs<'call>(
        &'call self,
        request: ListRunsArgs<'_>,
        log: EvalLogging<'call, '_>,
    ) -> Result<ListRunsWork<'call>, EvalRunFailure> {
        let provider = request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
        let call = match self.prepare_list(request, provider, log.state) {
            Ok(call) => call,
            Err(error) => return Err(list_failure(error, provider, &request, &self.redactor)),
        };
        let mode = call.mode;
        let pending = self.transport.list_runs(log, call);
        if mode == RunMode::Async {
            return Ok(ListRunsWork::Pending(pending));
        }
        match poll_once(pending) {
            Polled::Ready(Ok(runs)) => Ok(ListRunsWork::Ready(Box::new(runs))),
            Polled::Ready(Err(error)) => {
                Err(list_failure(error, provider, &request, &self.redactor))
            }
            Polled::StillWaiting => Err(list_failure(
                EvalRunCallError::Suspended,
                provider,
                &request,
                &self.redactor,
            )),
        }
    }

    // LiteLLM's alist_runs: the flag in kwargs, then list_runs with its coroutine awaited
    pub async fn alist_runs<'call>(
        &'call self,
        request: ListRunsArgs<'_>,
        log: EvalLogging<'call, '_>,
    ) -> Result<ListRunsResponse, EvalRunFailure> {
        let provider = request.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
        let mut kwargs = request.kwargs.clone();
        kwargs.insert(LIST_FLAG.to_owned(), Json::Bool(true));
        let flagged = ListRunsArgs {
            kwargs: &kwargs,
            ..request
        };
        match self.list_runs(flagged, log) {
            Ok(ListRunsWork::Ready(runs)) => Ok(*runs),
            Ok(ListRunsWork::Pending(pending)) => pending
                .await
                .map_err(|error| list_failure(error, provider, &flagged, &self.redactor)),
            Err(failure) => Err(failure),
        }
    }
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
