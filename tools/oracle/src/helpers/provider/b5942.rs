use std::collections::BTreeMap;

use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_openai::chat::{ChatMessage, Tool};
use betterlite_openai::stream::chunk::Choice;
use betterlite_prompts::arguments::json::Json;
use betterlite_provider::oci_chat_wire::cohere::reply::{
    CohereEmitted, handle_cohere_response, handle_cohere_stream_chunk,
};
use betterlite_provider::oci_chat_wire::cohere::request::{
    adapt_cohere_messages, adapt_cohere_tools,
};
use betterlite_provider::oci_chat_wire::generic::reply::{
    handle_generic_response, handle_generic_stream_chunk,
};
use betterlite_provider::oci_chat_wire::generic::request::{
    adapt_generic_messages, adapt_generic_tools,
};
use betterlite_provider::oci_chat_wire::ids::normalize_oci_finish_reason;
use betterlite_provider::oci_common::OciError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    GenericMessages {
        messages: Vec<ChatMessage>,
    },
    CohereMessages {
        messages: Vec<ChatMessage>,
    },
    GenericTools {
        tools: Vec<Tool>,
    },
    CohereTools {
        tools: Vec<Tool>,
    },
    GenericResponse {
        body: Json,
        status: u16,
    },
    CohereResponse {
        body: Json,
        model: String,
        created: u64,
        status: u16,
    },
    FinishReason {
        reasons: Vec<Option<String>>,
    },
    GenericStream {
        chunks: Vec<Json>,
    },
    CohereStream {
        calls: Vec<StreamCall>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamCall {
    chunk: Json,
    prior_tool_calls_emitted: bool,
    prior_text_emitted: bool,
}

fn oci_failure(error: OciError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "OciError".to_owned(),
        status: Some(error.status),
        error_type: None,
        param: None,
        code: None,
        message: error.message,
    })
}

fn encoded<T>(value: &T) -> crate::helpers::HelperResult
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|source| {
        Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: source.to_string(),
        })
    })
}

fn arguments_json(arguments: &str) -> Value {
    let Ok(value) = serde_json::from_str(arguments) else {
        return Value::String(arguments.to_owned());
    };
    value
}

fn tool_call_json(id: Option<&str>, kind: Option<&str>, function: Option<(&str, &str)>) -> Value {
    let (name, arguments) = function.unwrap_or_default();
    serde_json::json!({
        "id": id,
        "type": kind,
        "function": {"name": name, "arguments": arguments_json(arguments)},
    })
}

fn choice_json(choice: &Choice) -> Value {
    let tool_calls = choice.delta.tool_calls.as_ref().map(|calls| {
        calls
            .iter()
            .map(|call| {
                let function = call.function.as_ref().map(|function| {
                    (
                        function.name.as_deref().unwrap_or_default(),
                        function.arguments.as_str(),
                    )
                });
                tool_call_json(call.id.as_deref(), call.kind.as_deref(), function)
            })
            .collect::<Vec<Value>>()
    });
    serde_json::json!({
        "index": choice.index,
        "content": choice.delta.content,
        "tool_calls": tool_calls,
        "finish_reason": choice.finish_reason,
    })
}

fn response_json(response: &ChatResponse) -> Value {
    let choices = response
        .choices
        .iter()
        .map(|choice| {
            let tool_calls = choice.message.tool_calls.as_ref().map(|calls| {
                calls
                    .iter()
                    .map(|call| {
                        tool_call_json(
                            Some(call.id.as_str()),
                            Some(call.kind.as_str()),
                            Some((&call.function.name, &call.function.arguments)),
                        )
                    })
                    .collect::<Vec<Value>>()
            });
            serde_json::json!({
                "index": choice.index,
                "content": choice.message.content,
                "tool_calls": tool_calls,
                "finish_reason": choice.finish_reason,
            })
        })
        .collect::<Vec<Value>>();
    let details = response.usage.completion_tokens_details.as_ref();
    serde_json::json!({
        "created": response.created,
        "model": response.model,
        "choices": choices,
        "usage": {
            "prompt_tokens": response.usage.prompt_tokens,
            "completion_tokens": response.usage.completion_tokens,
            "total_tokens": response.usage.total_tokens,
            "reasoning_tokens": details.and_then(|given| given.get("reasoning_tokens")).copied().flatten(),
            "text_tokens": details.and_then(|given| given.get("text_tokens")).copied().flatten(),
        },
    })
}

fn base_response() -> ChatResponse {
    ChatResponse {
        id: String::from("chatcmpl-oci"),
        object: String::from("chat.completion"),
        created: 0,
        model: String::new(),
        choices: vec![ChatChoice {
            index: 0,
            message: AssistantMessage {
                role: String::from("assistant"),
                content: None,
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: None,
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        }],
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}

fn generic_response(body: &Json, status: u16) -> crate::helpers::HelperResult {
    let mut response = base_response();
    handle_generic_response(body, status, &mut response).map_err(oci_failure)?;
    Ok(response_json(&response))
}

fn cohere_response(
    body: &Json,
    model: &str,
    created: u64,
    status: u16,
) -> crate::helpers::HelperResult {
    let mut response = base_response();
    handle_cohere_response(body, model, created, status, &mut response).map_err(oci_failure)?;
    Ok(response_json(&response))
}

fn finish_reasons(reasons: &[Option<String>]) -> crate::helpers::HelperResult {
    let normalized = reasons
        .iter()
        .map(|reason| normalize_oci_finish_reason(reason.as_deref()))
        .collect::<Vec<Option<String>>>();
    encoded(&normalized)
}

fn generic_stream(chunks: &[Json]) -> crate::helpers::HelperResult {
    let mut choices = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        choices.push(choice_json(
            &handle_generic_stream_chunk(chunk.clone()).map_err(oci_failure)?,
        ));
    }
    Ok(Value::Array(choices))
}

fn cohere_stream(calls: &[StreamCall]) -> crate::helpers::HelperResult {
    let mut choices = Vec::with_capacity(calls.len());
    for call in calls {
        let choice = handle_cohere_stream_chunk(
            &call.chunk,
            CohereEmitted {
                tool_calls: call.prior_tool_calls_emitted,
                text: call.prior_text_emitted,
            },
        )
        .map_err(oci_failure)?;
        choices.push(choice_json(&choice));
    }
    Ok(Value::Array(choices))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::GenericMessages { messages } => {
                encoded(&adapt_generic_messages(messages).map_err(oci_failure)?)
            }
            Input::CohereMessages { messages } => encoded(&adapt_cohere_messages(messages)),
            Input::GenericTools { tools } => {
                encoded(&adapt_generic_tools(tools).map_err(oci_failure)?)
            }
            Input::CohereTools { tools } => {
                encoded(&adapt_cohere_tools(tools).map_err(oci_failure)?)
            }
            Input::GenericResponse { body, status } => generic_response(body, *status),
            Input::CohereResponse {
                body,
                model,
                created,
                status,
            } => cohere_response(body, model, *created, *status),
            Input::FinishReason { reasons } => finish_reasons(reasons),
            Input::GenericStream { chunks } => generic_stream(chunks),
            Input::CohereStream { calls } => cohere_stream(calls),
        }
    }
}
