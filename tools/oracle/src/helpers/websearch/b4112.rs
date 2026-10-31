use betterlite_openai::search::records::SearchResponse;
use betterlite_websearch::call_transformation::wire::{
    Assistant, Call, Response, Results, Thinking,
};
use betterlite_websearch::call_transformation::{WebSearchTransformation, native};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Detect,
    Native,
    Format,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    format: String,
    response: Option<Box<RawValue>>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    transform: bool,
    #[serde(default)]
    results: Vec<String>,
    #[serde(default)]
    thinking: Vec<Thinking>,
    search_response: Option<SearchResponse>,
}
#[derive(Serialize)]
struct Output {
    has_websearch: bool,
    tool_calls: Vec<Call>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assistant: Option<Assistant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    messages: Option<Results>,
}
impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.operation {
            Operation::Detect => detect(input),
            Operation::Native => {
                serde_json::to_value(native::result_block("call", input.search_response.as_ref()))
                    .map_err(failure)
            }
            Operation::Format => {
                let search = input
                    .search_response
                    .as_ref()
                    .ok_or_else(|| failure("missing search response"))?;
                let text = native::format_search_response(search).map_err(failure)?;
                serde_json::to_value(text).map_err(failure)
            }
        }
    }
}
fn detect(input: &Input) -> crate::helpers::HelperResult {
    let response: Response =
        serde_json::from_str(input.response.as_deref().map_or("{}", RawValue::get))
            .map_err(failure)?;
    let (has_websearch, tool_calls) =
        WebSearchTransformation::transform_request(&response, input.stream, &input.format)
            .map_err(failure)?;
    let (assistant, messages) = if input.transform {
        let (mut assistant_body, messages_body) =
            WebSearchTransformation::transform_response(&tool_calls, &input.results, &input.format)
                .map_err(failure)?;
        assistant_body.prepend_thinking(&input.thinking);
        (Some(assistant_body), Some(messages_body))
    } else {
        (None, None)
    };
    serde_json::to_value(Output {
        has_websearch,
        tool_calls,
        assistant,
        messages,
    })
    .map_err(failure)
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
