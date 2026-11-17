mod fixture;
mod view;

mod tests;

use std::sync::Arc;

use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamSuccessDispatch;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_logging::tinyfish::billing::{
    AsyncRun, BlockingRun, CollectedChunks, SseRun,
};
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::{Builder, Runtime};

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
        match input.function {
            Function::Blocking => blocking(input),
            Function::AsyncRun => async_run(input),
            Function::SseRun => sse_run(input),
            Function::CollectedChunks => collected_chunks(input),
        }
    }
}

// LiteLLM's blocking route bills from the response body before it answers, so one call is the whole path.
fn blocking(input: &Input) -> HelperResult {
    let fixture = fixture::fixture(Some(input.api_key.as_str()), Vec::new())?;
    let result = result_text(input)?;
    let kwargs = PassThroughLoggingKwargs::default();
    let outcome = fixture.billing.tinyfish_passthrough_handler(BlockingRun {
        logging_obj: fixture.logging_obj.as_ref(),
        response_body: input.run.as_ref(),
        result: &result,
        kwargs: &kwargs,
    });
    view::billing_view(
        fixture.logging_obj.as_ref(),
        view::outcome_logged(&outcome),
        fixture.transport.request_log(),
    )
}

fn async_run(input: &Input) -> HelperResult {
    let fixture = fixture::fixture(
        Some(input.api_key.as_str()),
        fixture::replies(&input.replies)?,
    )?;
    let runtime = runtime()?;
    let log: Arc<dyn StreamSuccessDispatch> = Arc::clone(&fixture.recorder);
    let handle = fixture.billing.start_async_run_billing(AsyncRun {
        logging_obj: Arc::clone(&fixture.logging_obj),
        response_body: input.run.as_ref(),
        result: result_text(input)?,
        kwargs: PassThroughLoggingKwargs::default(),
        log,
        start_time: fixture::start_time()?,
    });
    runtime.block_on(handle).or_value_error()?;
    polled_view(&fixture)
}

fn sse_run(input: &Input) -> HelperResult {
    let fixture = fixture::fixture(
        Some(input.api_key.as_str()),
        fixture::replies(&input.replies)?,
    )?;
    let runtime = runtime()?;
    let run_id = input
        .run_id
        .clone()
        .ok_or_else(|| failure("the sse_run function needs a run_id"))?;
    let log: Arc<dyn StreamSuccessDispatch> = Arc::clone(&fixture.recorder);
    let handle = fixture.billing.start_sse_run_billing(SseRun {
        run_id,
        logging_obj: Arc::clone(&fixture.logging_obj),
        log,
        start_time: fixture::start_time()?,
    });
    runtime.block_on(handle).or_value_error()?;
    polled_view(&fixture)
}

fn collected_chunks(input: &Input) -> HelperResult {
    let fixture = fixture::fixture(
        Some(input.api_key.as_str()),
        fixture::replies(&input.replies)?,
    )?;
    let runtime = runtime()?;
    let outcome = runtime.block_on(fixture.billing.handle_logging_tinyfish_collected_chunks(
        CollectedChunks {
            logging_obj: fixture.logging_obj.as_ref(),
            lines: &input.lines,
        },
    ));
    view::billing_view(
        fixture.logging_obj.as_ref(),
        view::outcome_logged(&outcome),
        fixture.transport.request_log(),
    )
}

fn polled_view(fixture: &fixture::Fixture) -> HelperResult {
    view::billing_view(
        fixture.logging_obj.as_ref(),
        fixture.recorder.only_call()?,
        fixture.transport.request_log(),
    )
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread().build().or_value_error()
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
