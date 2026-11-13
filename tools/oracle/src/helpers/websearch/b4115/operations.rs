use std::fmt::Display;

use betterlite_openai::search::records::SearchResponse;
use betterlite_websearch::call_transformation::outcome::SearchFailure;
use betterlite_websearch::call_transformation::wire::Input as ToolInput;
use betterlite_websearch::execution::rich_input::rich_search_input;
use betterlite_websearch::execution::short_circuit::ShortCircuitRequest;
use betterlite_websearch::records::RichWebSearchInput;
use serde::Serialize;
use tokio::runtime::Builder;

use super::Input;
use super::fakes::{self, RecordedCall};

#[derive(Serialize)]
struct ExecuteOutput {
    text: String,
    response: Option<SearchResponse>,
    calls: Vec<RecordedCall>,
}

pub(super) fn short_circuit(input: &Input) -> crate::helpers::HelperResult {
    let execution = fakes::build(input);
    let request = ShortCircuitRequest {
        model: &input.model,
        messages: &input.messages,
        tools: input.tools.as_deref(),
        custom_llm_provider: input.custom_llm_provider.as_deref(),
    };
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let response =
        runtime.block_on(execution.try_short_circuit_search(&request, input.kwargs.as_ref()));
    serde_json::to_value(response).map_err(failure)
}

pub(super) fn execute_search(input: &Input) -> crate::helpers::HelperResult {
    let execution = fakes::build(input);
    let rich = input
        .tool_inputs
        .first()
        .and_then(|raw| rich_search_input(&ToolInput::from(raw.clone())));
    let runtime = Builder::new_current_thread().build().map_err(failure)?;
    let outcome = runtime.block_on(execution.execute_search(
        &input.query,
        input.kwargs.as_ref(),
        rich.as_ref(),
    ));
    match outcome {
        Ok((text, response)) => {
            let output = ExecuteOutput {
                text,
                response,
                calls: fakes::recorded(&execution),
            };
            serde_json::to_value(output).map_err(failure)
        }
        Err(error) => Err(search_failure(error)),
    }
}

pub(super) fn rich_search_inputs(input: &Input) -> crate::helpers::HelperResult {
    let results: Vec<Option<RichWebSearchInput>> = input
        .tool_inputs
        .iter()
        .map(|raw| rich_search_input(&ToolInput::from(raw.clone())))
        .collect();
    serde_json::to_value(results).map_err(failure)
}

pub(super) fn provider_supports(input: &Input) -> crate::helpers::HelperResult {
    let execution = fakes::build(input);
    let results: Vec<bool> = input
        .search_providers
        .iter()
        .map(|provider| execution.provider_supports_rich_search(provider.as_deref()))
        .collect();
    serde_json::to_value(results).map_err(failure)
}

fn failure<Source>(error: Source) -> Box<crate::helpers::HelperError>
where
    Source: Display,
{
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "SearchExecutionError".to_owned(),
        message,
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}

fn search_failure(error: SearchFailure) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "SearchFailure".to_owned(),
        message: error.message,
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}
