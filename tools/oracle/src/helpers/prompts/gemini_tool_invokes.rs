use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::gemini_tool_invokes::invoke::{
    convert_tool_calls_to_gemini, gemini_tool_call_invoke_helper,
};
use betterlite_prompts::gemini_tool_invokes::protocol::infer_protocol_value;
use betterlite_prompts::gemini_tool_invokes::signatures::{
    DUMMY_THOUGHT_SIGNATURE, encode_tool_call_id, thought_signature_from_tool,
};
use betterlite_prompts::gemini_tool_invokes::types::{GeminiFunctionCallParams, GeminiTool};
use betterlite_prompts::message_content::Message;
use serde::Deserialize;
use serde_json::{json, to_value};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "infer_protocol_value")]
    Protocol { values: Vec<Json> },
    #[serde(rename = "gemini_tool_call_invoke_helper")]
    Invoke {
        function_call_params: GeminiFunctionCallParams,
        #[serde(default)]
        tool_call_id: Option<String>,
    },
    #[serde(rename = "encode_tool_call_id_with_signature")]
    Encode { pairs: Vec<IdPair> },
    #[serde(rename = "thought_signature_from_tool")]
    Signature { tools: Vec<GeminiTool> },
    #[serde(rename = "dummy_thought_signature")]
    Dummy,
    #[serde(rename = "convert_to_gemini_tool_call_invoke")]
    Convert {
        message: Message,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        forward_function_call_id: bool,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct IdPair {
    tool_call_id: String,
    #[serde(default)]
    thought_signature: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Protocol { values } => {
                ok_value(values.iter().map(infer_protocol_value).collect::<Vec<_>>())
            }
            Input::Invoke {
                function_call_params,
                tool_call_id,
            } => gemini_tool_call_invoke_helper(function_call_params, tool_call_id.as_deref())
                .map_err(|error| helper_error("GeminiToolInvokesError", error.to_string()))
                .and_then(ok_value),
            Input::Encode { pairs } => ok_value(
                pairs
                    .iter()
                    .map(|pair| {
                        encode_tool_call_id(&pair.tool_call_id, pair.thought_signature.as_deref())
                    })
                    .collect::<Vec<_>>(),
            ),
            Input::Signature { tools } => ok_value(
                tools
                    .iter()
                    .map(thought_signature_from_tool)
                    .collect::<Vec<_>>(),
            ),
            Input::Dummy => Ok(json!(DUMMY_THOUGHT_SIGNATURE)),
            Input::Convert {
                message,
                model,
                forward_function_call_id,
            } => convert_tool_calls_to_gemini(message, model.as_deref(), *forward_function_call_id)
                .map_err(|error| helper_error("GeminiToolInvokesError", error.to_string()))
                .and_then(ok_value),
        }
    }
}

fn ok_value<Payload>(payload: Payload) -> HelperResult
where
    Payload: serde::Serialize,
{
    match to_value(payload) {
        Ok(value) => Ok(value),
        Err(error) => Err(helper_error("SerializationError", error.to_string())),
    }
}

fn helper_error(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
