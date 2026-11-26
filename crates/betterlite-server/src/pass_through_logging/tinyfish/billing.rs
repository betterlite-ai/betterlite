mod poller;

use std::sync::Arc;

use betterlite_logging::logging_success::ports::SuccessClock;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use tokio::task::JoinHandle;

use super::error::TinyfishError;
use super::fetch::{RunCredentials, RunTransport, fetch_run};
use super::payload::build_logging_payload;
use super::poll::{PollClock, PollSleeper};
use super::records::{TinyfishRun, parse_run};
use super::sse::{mark_sse_poller_spawned, run_id_from_sse_lines};
use crate::pass_through_logging::background_tasks::BackgroundTasks;

// The base and the rate travel with each billing call, so a proxy restart is not needed to move either.
#[derive(Clone, Debug)]
pub struct RunSettings {
    pub api_base: String,
    pub cost_per_step: Decimal,
}

#[derive(Clone, Copy)]
pub struct PayloadInput<'call> {
    pub logging_obj: &'call StreamLoggingObject,
    pub run: Option<&'call TinyfishRun>,
    pub result: &'call str,
    pub kwargs: &'call PassThroughLoggingKwargs,
    pub cost_per_step: Decimal,
}

// LiteLLM wraps the payload build in a blanket except, so a build that fails must still leave a usable outcome.
pub trait RunPayload: Send + Sync {
    fn build(&self, input: PayloadInput<'_>) -> Result<HandlerOutcome, TinyfishError>;
}

pub struct ProductionPayload;

impl RunPayload for ProductionPayload {
    fn build(&self, input: PayloadInput<'_>) -> Result<HandlerOutcome, TinyfishError> {
        build_logging_payload(
            input.logging_obj,
            input.run,
            input.result,
            input.kwargs,
            input.cost_per_step,
        )
    }
}

pub struct BillingDeps {
    pub credentials: Arc<dyn RunCredentials>,
    pub transport: Arc<dyn RunTransport>,
    pub monotonic: Arc<dyn PollClock>,
    pub sleeper: Arc<dyn PollSleeper>,
    pub clock: Arc<dyn SuccessClock>,
    pub payload: Arc<dyn RunPayload>,
    pub tasks: BackgroundTasks,
}

pub struct TinyfishRunBilling {
    settings: RunSettings,
    deps: Arc<BillingDeps>,
}

#[derive(Clone, Copy)]
pub struct BlockingRun<'run> {
    pub logging_obj: &'run StreamLoggingObject,
    pub response_body: Option<&'run Json>,
    pub result: &'run str,
    pub kwargs: &'run PassThroughLoggingKwargs,
}

pub struct AsyncRun<'run> {
    pub logging_obj: Arc<StreamLoggingObject>,
    pub response_body: Option<&'run Json>,
    pub result: String,
    pub kwargs: PassThroughLoggingKwargs,
    pub log: Arc<dyn StreamSuccessDispatch>,
    pub start_time: DateTime<FixedOffset>,
}

pub struct SseRun {
    pub run_id: String,
    pub logging_obj: Arc<StreamLoggingObject>,
    pub log: Arc<dyn StreamSuccessDispatch>,
    pub start_time: DateTime<FixedOffset>,
}

#[derive(Clone, Copy)]
pub struct CollectedChunks<'run> {
    pub logging_obj: &'run StreamLoggingObject,
    pub lines: &'run [String],
}

// The poller paths pass no request kwargs, so the stored params and times are all a billing call has.
struct PollRun {
    run_id: Option<String>,
    logging_obj: Arc<StreamLoggingObject>,
    result: String,
    kwargs: PassThroughLoggingKwargs,
    log: Arc<dyn StreamSuccessDispatch>,
    start_time: DateTime<FixedOffset>,
}

impl TinyfishRunBilling {
    #[must_use]
    pub fn new(settings: RunSettings, deps: BillingDeps) -> Self {
        Self {
            settings,
            deps: Arc::new(deps),
        }
    }

    // The blocking route's response is the terminal run, so the payload is built before the request is answered.
    #[must_use]
    pub fn tinyfish_passthrough_handler(&self, input: BlockingRun<'_>) -> HandlerOutcome {
        let run = input.response_body.and_then(parse_run);
        let built = self.deps.payload.build(PayloadInput {
            logging_obj: input.logging_obj,
            run: run.as_ref(),
            result: input.result,
            kwargs: input.kwargs,
            cost_per_step: self.settings.cost_per_step,
        });
        match built {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::error!(%error, "Error in TinyFish passthrough logging handler");
                HandlerOutcome {
                    result: Some(PassThroughResponse::Standard(PassThroughResponseObject {
                        response: Json::Text(input.result.to_owned()),
                    })),
                    kwargs: input.kwargs.clone(),
                }
            }
        }
    }

    // A run-async answer is pending, so the charge lands only when a detached poller sees the run finish.
    #[must_use]
    pub fn start_async_run_billing(&self, input: AsyncRun<'_>) -> JoinHandle<()> {
        let submitted = input.response_body.and_then(parse_run);
        let run_id = submitted.and_then(|run| run.run_id).flatten();
        if run_id.as_deref().is_none_or(str::is_empty) {
            tracing::warn!(
                "TinyFish passthrough: run-async response carried no run_id; logging the request without cost"
            );
        }
        poller::spawn_poll(
            &self.deps,
            &self.settings,
            PollRun {
                run_id,
                logging_obj: input.logging_obj,
                result: input.result,
                kwargs: input.kwargs,
                log: input.log,
                start_time: input.start_time,
            },
        )
    }

    // The marker goes down before the spawn, so the end of the stream sees that a poller owns this request.
    #[must_use]
    pub fn start_sse_run_billing(&self, input: SseRun) -> JoinHandle<()> {
        mark_sse_poller_spawned(&input.logging_obj);
        poller::spawn_poll(
            &self.deps,
            &self.settings,
            PollRun {
                run_id: Some(input.run_id),
                logging_obj: input.logging_obj,
                result: String::new(),
                kwargs: PassThroughLoggingKwargs::default(),
                log: input.log,
                start_time: input.start_time,
            },
        )
    }

    // The no-poller fallback; a run_id found in the collected frames still prices the run with one GET.
    #[must_use]
    // ast-grep-ignore: no-long-names
    pub async fn handle_logging_tinyfish_collected_chunks(
        &self,
        call: CollectedChunks<'_>,
    ) -> HandlerOutcome {
        let run_id = run_id_from_sse_lines(call.lines);
        if run_id.is_none() {
            tracing::warn!(
                "TinyFish passthrough: no run_id in SSE stream; logging the request without cost"
            );
        }
        let run = match run_id {
            Some(found) => {
                fetch_run(
                    self.deps.credentials.as_ref(),
                    self.deps.transport.as_ref(),
                    &self.settings.api_base,
                    &found,
                )
                .await
            }
            None => None,
        };
        let empty = PassThroughLoggingKwargs::default();
        let built = self.deps.payload.build(PayloadInput {
            logging_obj: call.logging_obj,
            run: run.as_ref(),
            result: "",
            kwargs: &empty,
            cost_per_step: self.settings.cost_per_step,
        });
        match built {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::error!(%error, "Error in TinyFish SSE passthrough logging handler");
                HandlerOutcome {
                    result: Some(PassThroughResponse::Standard(PassThroughResponseObject {
                        response: Json::Text(String::new()),
                    })),
                    kwargs: PassThroughLoggingKwargs::default(),
                }
            }
        }
    }
}
