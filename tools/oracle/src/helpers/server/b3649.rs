mod fixture;
mod ports;
mod view;

mod tests;

use std::sync::Arc;

use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_logging::tinyfish::billing::{
    AsyncRun, BlockingRun, CollectedChunks, SseRun,
};
use fixture::Fixture;
use serde::Deserialize;
use tokio::runtime::{Builder, Runtime};
use tokio::task::JoinHandle;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    api_key: String,
    #[serde(default)]
    run: Option<Json>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    replies: Vec<Reply>,
    #[serde(default)]
    lines: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    status: u16,
    body: Json,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Blocking,
    AsyncRun,
    SseRun,
    CollectedChunks,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let replies = match input.function {
            Function::Blocking => Vec::new(),
            Function::AsyncRun | Function::SseRun | Function::CollectedChunks => {
                fixture::replies(&input.replies)?
            }
        };
        let fixture = fixture::fixture(Some(input.api_key.as_str()), replies)?;
        dispatch(input, &fixture)
    }
}

// Each route bills from its own point: run-async and run-sse spawn a poller, while the other two return their payload.
fn dispatch(input: &Input, fixture: &Fixture) -> HelperResult {
    let runtime = Builder::new_current_thread().build().or_value_error()?;
    match input.function {
        Function::AsyncRun => {
            let result = result_text(input)?;
            let log = Arc::clone(&fixture.recorder);
            polled_answer(fixture, &runtime, || {
                fixture.billing.start_async_run_billing(AsyncRun {
                    logging_obj: Arc::clone(&fixture.logging_obj),
                    response_body: input.run.as_ref(),
                    result,
                    kwargs: PassThroughLoggingKwargs::default(),
                    log,
                    start_time: fixture.start_time,
                })
            })
        }
        Function::SseRun => {
            let run_id = input
                .run_id
                .clone()
                .ok_or_else(|| failure("the sse_run function needs a run_id"))?;
            let log = Arc::clone(&fixture.recorder);
            polled_answer(fixture, &runtime, || {
                fixture.billing.start_sse_run_billing(SseRun {
                    run_id,
                    logging_obj: Arc::clone(&fixture.logging_obj),
                    log,
                    start_time: fixture.start_time,
                })
            })
        }
        Function::Blocking => blocking_run(input, fixture),
        Function::CollectedChunks => collected_chunks(input, fixture, &runtime),
    }
}

fn blocking_run(input: &Input, fixture: &Fixture) -> HelperResult {
    let result = result_text(input)?;
    let kwargs = PassThroughLoggingKwargs::default();
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: fixture.logging_obj.as_ref(),
        response_body: input.run.as_ref(),
        result: &result,
        kwargs: &kwargs,
    });
    payload_view(fixture, &outcome)
}

fn collected_chunks(input: &Input, fixture: &Fixture, runtime: &Runtime) -> HelperResult {
    let outcome = runtime.block_on(fixture.billing.handle_logging_tinyfish_collected_chunks(
        CollectedChunks {
            logging_obj: fixture.logging_obj.as_ref(),
            lines: &input.lines,
        },
    ));
    payload_view(fixture, &outcome)
}

fn payload_view(fixture: &Fixture, outcome: &HandlerOutcome) -> HelperResult {
    let logged = view::outcome_logged(outcome);
    view::billing_view(
        fixture.logging_obj.as_ref(),
        &logged,
        fixture.transport.request_log(),
    )
}

// The poller is spawned inside the runtime above, which then drives it to its terminal run here.
fn polled_answer<Start>(fixture: &Fixture, runtime: &Runtime, start: Start) -> HelperResult
where
    Start: FnOnce() -> JoinHandle<()>,
{
    let handle = {
        let _entered = runtime.enter();
        start()
    };
    runtime.block_on(handle).or_value_error()?;
    let logged = fixture.recorder.only_call()?;
    view::billing_view(
        fixture.logging_obj.as_ref(),
        &logged,
        fixture.transport.request_log(),
    )
}

// The route hands LiteLLM the raw submit body, which is what a run that never turns terminal is logged with.
fn result_text(input: &Input) -> Result<String, Box<HelperError>> {
    if let Some(text) = input.result.as_ref() {
        return Ok(text.clone());
    }
    match input.run.as_ref() {
        Some(run) => serde_json::to_string(run).or_value_error(),
        None => Ok(String::new()),
    }
}

pub(super) fn failure(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "B3649".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
