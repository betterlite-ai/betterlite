mod fakes;
mod input;
mod log;

use std::str::from_utf8;
use std::sync::{Arc, Mutex};

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::stream_usage_cost::StreamCostInjector;
use indexmap::IndexMap;

use fakes::{FakeCalculator, FakeRecorder, price, take};
use input::{ChunkKind, Input};
use log::{call_log, failure};

const DEBUG_KEY: &str = "response_cost_failure_debug_information";

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let recorder = FakeRecorder {
            cost: price(input.prices.recorder.as_ref()),
            observed: Arc::clone(&observed),
        };
        let calculator = FakeCalculator {
            cost: price(input.prices.completion.as_ref()),
            observed: Arc::clone(&observed),
        };
        let injector = StreamCostInjector {
            enabled: input.enabled,
            recorder: &recorder,
            calculator: &calculator,
        };
        let mut log = match &input.log {
            Some(fields) => Some(call_log(fields)?),
            None => None,
        };
        let result = match input.chunk.kind {
            ChunkKind::Object => {
                let Some(value) = &input.chunk.value else {
                    return Err(failure("the object chunk needs a value".to_owned()));
                };
                injector
                    .process_object(
                        &Json::Object(value.clone()),
                        &input.model_name,
                        log.as_mut(),
                    )
                    .into_owned()
            }
            ChunkKind::Text => {
                let Some(text) = &input.chunk.text else {
                    return Err(failure("the text chunk needs a text".to_owned()));
                };
                Json::Text(
                    injector
                        .process_text(text, &input.model_name, log.as_mut())
                        .into_owned(),
                )
            }
            ChunkKind::Bytes => {
                let Some(text) = &input.chunk.text else {
                    return Err(failure("the bytes chunk needs a text".to_owned()));
                };
                bytes_result(injector, text, input, log.as_mut())?
            }
        };
        let mut output = IndexMap::new();
        output.insert("result".to_owned(), result);
        output.insert(
            "debug".to_owned(),
            match &log {
                Some(state) => state
                    .model_call_details
                    .get(DEBUG_KEY)
                    .cloned()
                    .unwrap_or(Json::Null),
                None => Json::Null,
            },
        );
        output.insert(
            "debug_present".to_owned(),
            Json::Bool(
                log.as_ref()
                    .is_some_and(|state| state.model_call_details.contains_key(DEBUG_KEY)),
            ),
        );
        output.insert("calls".to_owned(), Json::List(take(&observed)));
        let value = serde_json::to_value(Json::Object(output))
            .map_err(|error| failure(error.to_string()))?;
        Ok(value)
    }
}

fn bytes_result(
    injector: StreamCostInjector<'_>,
    text: &str,
    input: &Input,
    log: Option<&mut LoggingState>,
) -> Result<Json, Box<HelperError>> {
    let output = injector.process_bytes(text.as_bytes(), &input.model_name, log);
    let decoded = from_utf8(&output).map_err(|error| failure(error.to_string()))?;
    Ok(Json::Text(decoded.to_owned()))
}
