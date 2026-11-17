use std::convert::Infallible;
use std::time::SystemTime;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::agentcore_stream::IdSource;
use crate::converse_response::request::{OptionalParams, RawReply, ReplyRequest};
use crate::converse_response::transform::{TransformCall, TransformError, transform_response};
use crate::converse_response::wire::{Choice, ModelResponse, ReplyToolCall};
use crate::converse_usage::transform::ReasoningTokenCounter;
use crate::image_call::plan::LogContext;
use crate::invoke_events::fake::{
    FakeArguments, FakeChoice, FakeFunction, FakeMessage, FakeResponse, FakeStream, FakeToolCall,
    FakeUsage,
};

pub struct FakeCall<'call> {
    pub model: &'call str,
    pub messages: &'call Json,
    pub data: &'call str,
    pub json_mode: Option<bool>,
    pub now: SystemTime,
}

// _transform_response runs on the buffered body with an empty optional_params and an empty api_key.
pub fn buffered(
    call: FakeCall<'_>,
    reply: &RawReply<'_>,
    log: &mut LogContext<'_, '_>,
    ids: &dyn IdSource,
) -> Result<FakeStream, TransformError<Infallible>> {
    let optional = OptionalParams::default();
    let mappings = IndexMap::new();
    let request = ReplyRequest {
        model: call.model,
        messages: call.messages,
        data: RawBody::Text(call.data.to_owned()),
        json_mode: None,
        optional: &optional,
        tool_name_mappings: &mappings,
    };
    let response = fresh_response(ids);
    let context = LogContext {
        state: &mut *log.state,
        env: log.env,
    };
    let transformed = transform_response(
        TransformCall {
            request,
            log: context,
            now: call.now,
        },
        reply,
        response,
        ids,
        &NoReasoningTokens,
    )?;
    Ok(FakeStream {
        response: view(&transformed),
        json_mode: call.json_mode,
        is_done: false,
    })
}

// The ModelResponse a buffered conversion starts from: no choices and no usage, filled in by the converter.
fn fresh_response(ids: &dyn IdSource) -> ModelResponse {
    ModelResponse {
        id: format!("chatcmpl-{}", ids.next_id()),
        created: 0,
        model: String::new(),
        object: "chat.completion".to_owned(),
        system_fingerprint: None,
        choices: Vec::new(),
        usage: None,
        service_tier: None,
        trace: None,
    }
}

// The fake path owns no tokenizer, so it answers every reasoning estimate with the zero deviation B691 records.
struct NoReasoningTokens;

impl ReasoningTokenCounter for NoReasoningTokens {
    type Failure = Infallible;

    // No fixture in the crate feeds a fake reply with reasoning content, so no test reaches this answer.
    #[cfg_attr(test, mutants::skip)]
    fn count_reasoning_tokens(&self, _text: &str) -> Result<i64, Self::Failure> {
        Ok(0)
    }
}

fn view(response: &ModelResponse) -> FakeResponse {
    FakeResponse {
        choices: response.choices.iter().map(choice_view).collect(),
        usage: response.usage.as_ref().map(|usage| FakeUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        }),
    }
}

fn choice_view(choice: &Choice) -> FakeChoice {
    FakeChoice {
        finish_reason: Some(choice.finish_reason.clone()),
        message: FakeMessage {
            content: choice.message.content.clone(),
            tool_calls: choice
                .message
                .tool_calls
                .as_deref()
                .map(|calls| calls.iter().map(call_view).collect()),
        },
    }
}

fn call_view(call: &ReplyToolCall) -> FakeToolCall {
    let (id, name, arguments) = match call {
        ReplyToolCall::Content(tool) => (
            tool.id.clone(),
            tool.function.name.clone(),
            tool.function.arguments.clone(),
        ),
        ReplyToolCall::Recovered(tool) => (
            Some(tool.id.clone()),
            tool.function.name.clone(),
            Some(tool.function.arguments.clone()),
        ),
    };
    FakeToolCall {
        id,
        function: FakeFunction {
            name,
            arguments: arguments.map(FakeArguments::Text),
        },
    }
}
