use betterlite_error::error::ErrorClass;
use betterlite_openai::search::records::SearchResponse;
use betterlite_websearch::call_transformation::outcome::{
    SearchFailure, SearchResult, search_outcome,
};
use betterlite_websearch::call_transformation::wire::{Response, Thinking};
use betterlite_websearch::call_transformation::{WebSearchTransformation, native};
use betterlite_websearch::native_blocks::{self, ResponseJson};
use betterlite_websearch::records::{SearchOutcome, WebSearchToolResultErrorCode};
use serde::Deserialize;
use serde_json::value::RawValue;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    SearchErrorCode,
    SearchOutcome,
    FormatSearchResponse,
    TransformResponse,
    BuildWebSearchToolResultBlock,
    #[serde(rename = "build_web_search_tool_result_error_block")]
    BuildResultErrorBlock,
    NativeResultPairs,
    InjectNativeBlocks,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    error: Option<ErrorClass>,
    text: Option<String>,
    search_response: Option<SearchResponse>,
    message: Option<String>,
    response: Option<Box<RawValue>>,
    #[serde(default = "default_format")]
    format: String,
    #[serde(default)]
    search_results: Vec<String>,
    #[serde(default)]
    thinking: Vec<Thinking>,
    tool_use_id: Option<String>,
    error_code: Option<WebSearchToolResultErrorCode>,
    #[serde(default)]
    outcomes: Vec<SearchOutcome>,
    #[serde(default)]
    native_blocks: Vec<ResponseJson>,
}

fn default_format() -> String {
    "anthropic".to_owned()
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.operation {
            Operation::SearchErrorCode => search_error_code_value(input),
            Operation::SearchOutcome => search_outcome_value(input),
            Operation::FormatSearchResponse => format_search_response_value(input),
            Operation::TransformResponse => transform_response_value(input),
            Operation::BuildWebSearchToolResultBlock => result_block_value(input),
            Operation::BuildResultErrorBlock => result_error_block_value(input),
            Operation::NativeResultPairs => native_result_pairs_value(input),
            Operation::InjectNativeBlocks => inject_native_blocks_value(input),
        }
    }
}

fn required_field<'input, Value>(
    value: Option<&'input Value>,
    field: &'static str,
) -> Result<&'input Value, Box<crate::helpers::HelperError>>
where
    Value: ?Sized,
{
    let Some(found) = value else {
        return Err(failure(format!("missing required field {field}")));
    };
    Ok(found)
}

fn search_error_code_value(input: &Input) -> crate::helpers::HelperResult {
    let error = required_field(input.error.as_ref(), "error")?;
    serde_json::to_value(native::search_error_code(error)).map_err(failure)
}

fn format_search_response_value(input: &Input) -> crate::helpers::HelperResult {
    let search_response = required_field(input.search_response.as_ref(), "search_response")?;
    let text = native::format_search_response(search_response).map_err(failure)?;
    serde_json::to_value(text).map_err(failure)
}

fn result_block_value(input: &Input) -> crate::helpers::HelperResult {
    let tool_use_id = required_field(input.tool_use_id.as_deref(), "tool_use_id")?;
    let block = native::result_block(tool_use_id, input.search_response.as_ref());
    serde_json::to_value(block).map_err(failure)
}

fn result_error_block_value(input: &Input) -> crate::helpers::HelperResult {
    let tool_use_id = required_field(input.tool_use_id.as_deref(), "tool_use_id")?;
    let error_code = *required_field(input.error_code.as_ref(), "error_code")?;
    let block = native::result_error_block(tool_use_id, error_code);
    serde_json::to_value(block).map_err(failure)
}

fn inject_native_blocks_value(input: &Input) -> crate::helpers::HelperResult {
    let raw_response = required_field(input.response.as_deref(), "response")?;
    let response: ResponseJson = serde_json::from_str(raw_response.get()).map_err(failure)?;
    let patched = native_blocks::inject_native_blocks(response, &input.native_blocks);
    Ok(patched)
}

fn search_outcome_value(input: &Input) -> crate::helpers::HelperResult {
    let text = input.text.clone().unwrap_or_default();
    let message = input.message.clone().unwrap_or_default();
    let result = input.error.as_ref().map_or_else(
        || SearchResult::Succeeded {
            text,
            response: input.search_response.clone(),
        },
        |class| {
            SearchResult::Failed(SearchFailure {
                class: class.clone(),
                message,
            })
        },
    );
    serde_json::to_value(search_outcome(result)).map_err(failure)
}

fn transform_response_value(input: &Input) -> crate::helpers::HelperResult {
    let raw_response = required_field(input.response.as_deref(), "response")?;
    let response: Response = serde_json::from_str(raw_response.get()).map_err(failure)?;
    let (_, calls) = WebSearchTransformation::transform_request(&response, false, &input.format)
        .map_err(failure)?;
    let (mut assistant, messages) =
        WebSearchTransformation::transform_response(&calls, &input.search_results, &input.format)
            .map_err(failure)?;
    assistant.prepend_thinking(&input.thinking);
    serde_json::to_value((assistant, messages)).map_err(failure)
}

fn native_result_pairs_value(input: &Input) -> crate::helpers::HelperResult {
    let raw_response = required_field(input.response.as_deref(), "response")?;
    let format = &input.format;
    let outcomes = &input.outcomes;
    let response: Response = serde_json::from_str(raw_response.get()).map_err(failure)?;
    let (_, calls) =
        WebSearchTransformation::transform_request(&response, false, format).map_err(failure)?;
    let pairs = native_blocks::build_native_result_blocks(&calls, outcomes).map_err(failure)?;
    let blocks: Vec<serde_json::Value> = pairs
        .into_iter()
        .map(|(tool_use, result)| {
            Ok([
                serde_json::to_value(tool_use)?,
                serde_json::to_value(result)?,
            ])
        })
        .collect::<Result<Vec<[serde_json::Value; 2]>, serde_json::Error>>()
        .map_err(failure)?
        .into_iter()
        .flatten()
        .collect();
    Ok(serde_json::Value::Array(blocks))
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "TransformationError".to_owned(),
        message,
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}
