use std::sync::{Arc, PoisonError};

use betterlite_logging::logging_success::types::SuccessResult;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use tokio::task::JoinHandle;

use super::super::BILLING_CANCELLED_WARNING;
use super::super::poll::Poller;
use super::super::records::TinyfishRun;
use super::{BillingDeps, PayloadInput, PollRun, RunSettings};

struct RunLog {
    logging_obj: Arc<StreamLoggingObject>,
    log: Arc<dyn StreamSuccessDispatch>,
    outcome: HandlerOutcome,
    start_time: DateTime<FixedOffset>,
    end_time: DateTime<FixedOffset>,
}

// The poller runs detached from the request, so the server's task tracker owns it and the drain can wait for it.
pub(super) fn spawn_poll(
    deps: &Arc<BillingDeps>,
    settings: &RunSettings,
    poll: PollRun,
) -> JoinHandle<()> {
    let polled = Arc::clone(deps);
    let run_settings = settings.clone();
    deps.tasks.spawn(
        async move { poll_and_log(&polled, &run_settings, poll).await },
        BILLING_CANCELLED_WARNING.to_owned(),
    )
}

// Billing must never break the relayed response, so every failure here ends in a log line and no spend row.
async fn poll_and_log(deps: &BillingDeps, settings: &RunSettings, poll: PollRun) {
    let poller = Poller {
        credentials: deps.credentials.as_ref(),
        transport: deps.transport.as_ref(),
        clock: deps.monotonic.as_ref(),
        sleeper: deps.sleeper.as_ref(),
        api_base: &settings.api_base,
    };
    let run: Option<TinyfishRun> = match poll.run_id.as_deref().filter(|found| !found.is_empty()) {
        Some(run_id) => poller.poll_until_terminal(run_id).await,
        None => None,
    };
    let end_time = deps.clock.now();
    let built = deps.payload.build(PayloadInput {
        logging_obj: &poll.logging_obj,
        run: run.as_ref(),
        result: &poll.result,
        kwargs: &poll.kwargs,
        cost_per_step: settings.cost_per_step,
    });
    let outcome = match built {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::error!(
                %error,
                "[Non blocking logging error] TinyFish run-async billing failed"
            );
            return;
        }
    };
    dispatch_run_log(RunLog {
        logging_obj: Arc::clone(&poll.logging_obj),
        log: poll.log,
        outcome,
        start_time: poll.start_time,
        end_time,
    })
    .await;
}

// LiteLLM's _handle_logging always logs a poller's row with cache_hit false, whatever the request carried.
async fn dispatch_run_log(call: RunLog) {
    let taken = call
        .logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let Some(mut state) = taken else {
        tracing::error!("the TinyFish logging state was taken before the billing dispatch");
        return;
    };
    let result = call
        .outcome
        .result
        .map_or_else(|| SuccessResult::Other(Json::Null), SuccessResult::from);
    call.log
        .dispatch_success(StreamSuccessCall {
            state: &mut state.logging,
            correlation: &mut state.correlation,
            result,
            start_time: call.start_time,
            end_time: call.end_time,
            cache_hit: false,
            kwargs: &call.outcome.kwargs,
        })
        .await;
    *call
        .logging_obj
        .slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(state);
}
