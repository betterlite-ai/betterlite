mod fixture;
mod view;

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::cost::recording::PortFailure;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_logging::anthropic_batch::{
    AnthropicBatchLogging, ResponseIds,
};
use betterlite_server::pass_through_logging::anthropic_passthrough_logging::{
    AnthropicPassthroughInput, AnthropicPassthroughLogger,
};
use betterlite_server::pass_through_logging::response_costing::{
    ResponseCostRequest, ResponseCostService,
};
use http::HeaderMap;
use rust_decimal::Decimal;
use serde::Deserialize;

pub(crate) struct Op;

// Every field is always given, so the Python TypedDict and this struct read the same case.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    status_code: u16,
    url_route: String,
    body: String,
    request_body: JsonObject,
    litellm_params: JsonObject,
    passthrough_logging_payload: JsonObject,
    details: JsonObject,
    lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Handler,
    Chunks,
}

// The Python side patches litellm.completion_cost with the same fixed price and records the same fields.
struct RecordingCost {
    calls: Mutex<Vec<view::PricedCall>>,
}

impl ResponseCostService for RecordingCost {
    fn completion_cost(&self, request: &ResponseCostRequest<'_>) -> Result<Decimal, PortFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(view::PricedCall::of(request));
        Ok(Decimal::new(125, 2))
    }
}

struct FixedIds;

impl ResponseIds for FixedIds {
    fn new_response_id(&self) -> String {
        view::BATCH_RESPONSE_ID.to_owned()
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cost = Arc::new(RecordingCost {
            calls: Mutex::new(Vec::new()),
        });
        let shared: Arc<dyn ResponseCostService> = Arc::<RecordingCost>::clone(&cost);
        let logger = AnthropicPassthroughLogger::new(
            AnthropicBatchLogging::new(None, None, Arc::new(FixedIds)),
            shared,
        );
        let logging_obj = fixture::logging_object(&input.details)?;
        let outcome = match input.operation {
            Operation::Handler => handler_outcome(&logger, input, &logging_obj)?,
            Operation::Chunks => fixture::chunks_outcome(&logger, input, &logging_obj)?,
        };
        let calls = cost.calls.lock().unwrap_or_else(PoisonError::into_inner);
        view::outcome_view(&outcome, &logging_obj, &calls)
    }
}

fn handler_outcome(
    logger: &AnthropicPassthroughLogger,
    input: &Input,
    logging_obj: &StreamLoggingObject,
) -> Result<HandlerOutcome, Box<HelperError>> {
    let headers = HeaderMap::new();
    // success_handler's shaping: the body as an object, or an empty one for a list or no JSON
    let response_body: JsonObject = serde_json::from_str(&input.body).unwrap_or_default();
    let start_time = fixture::start_time()?;
    let mut handled = None;
    logging_obj.edit(&mut |call_log| {
        handled = Some(logger.anthropic_passthrough_handler(
            AnthropicPassthroughInput {
                status_code: input.status_code,
                response_headers: &headers,
                response_text: &input.body,
                response_body: &response_body,
                url_route: &input.url_route,
                start_time,
                end_time: start_time,
                request_body: &input.request_body,
                kwargs: logged_kwargs(input),
            },
            call_log,
        ));
    });
    handled
        .ok_or("the fixture call log was not in its slot")
        .or_value_error()?
        .or_value_error()
}

// The case's litellm_params and passthrough payload are the two kwargs entries the payload step reads.
fn logged_kwargs(input: &Input) -> PassThroughLoggingKwargs {
    let mut kwargs = PassThroughLoggingKwargs::default();
    kwargs.entries.insert(
        "litellm_params".to_owned(),
        Json::Object(input.litellm_params.clone()),
    );
    kwargs.entries.insert(
        "passthrough_logging_payload".to_owned(),
        Json::Object(input.passthrough_logging_payload.clone()),
    );
    kwargs
}
