use std::collections::BTreeMap;

use betterlite_openai::api::responses_shapes::builders::{
    Field, InterpreterContent, SearchInput, SearchResult, WebSearchStatus, build_interpreter_logs,
    build_web_search_call,
};
use betterlite_openai::api::responses_shapes::websocket::WsRequestDefaults;
use betterlite_openai::api::responses_shapes::{
    CustomToolCallOutputItem, DecodedResponseId, DeleteResponseResult, GenericResponseOutputItem,
    OutputCodeInterpreterCall, OutputFunctionToolCall, OutputImageGenerationCall, OutputText,
};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    WebSearchSources {
        tool_id: String,
        tool_input: Field<SearchInput>,
        result: Field<SearchResult>,
        status: Option<WebSearchStatus>,
    },
    InterpreterLogs {
        content: Field<InterpreterContent>,
    },
    WebsocketDefaults {
        #[serde(flatten)]
        defaults: WsRequestDefaults,
        request: BTreeMap<String, Box<RawValue>>,
    },
    NullableRecords(Box<Records>),
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Records {
    record: DeleteResponseResult,
    output_text: OutputText,
    function_call: OutputFunctionToolCall,
    image_call: OutputImageGenerationCall,
    interpreter_call: OutputCodeInterpreterCall,
    custom_call: CustomToolCallOutputItem,
    generic_item: GenericResponseOutputItem,
    decoded_id: DecodedResponseId,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::WebSearchSources {
                tool_id,
                tool_input,
                result,
                status,
            } => serde_json::to_value(build_web_search_call(tool_id, tool_input, result, *status)),
            Input::InterpreterLogs { content } => {
                serde_json::to_value(build_interpreter_logs(content))
            }
            Input::WebsocketDefaults { defaults, request } => {
                serde_json::to_value(defaults.merged_into(request))
            }
            Input::NullableRecords(records) => serde_json::to_value(records),
        };
        output.map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
