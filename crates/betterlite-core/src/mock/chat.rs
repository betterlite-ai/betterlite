use std::collections::BTreeMap;

use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::MockPlan;
use betterlite_config::sections::mock_response::{
    MockChatChoice, MockChatObject, MockResponse, UsageDefaults,
};
use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_openai::chat::{FunctionCall, ToolCall};

// LiteLLM's mock_completion answer when the caller and the operator both leave mock_response unset
pub const DEFAULT_CONTENT: &str = "This is a mock request";

pub enum ChatAnswer<'content> {
    Content(&'content str),
    Object(&'content MockChatObject),
}

pub fn answer(response: Option<&MockResponse>) -> Result<ChatAnswer<'_>, MockError> {
    match response {
        None => Ok(ChatAnswer::Content(DEFAULT_CONTENT)),
        Some(MockResponse::Text(text)) => content(text),
        Some(MockResponse::Object(object)) => Ok(ChatAnswer::Object(object)),
        Some(MockResponse::Numbers(_)) => Err(MockError::WrongType {
            field: "mock_response",
        }),
    }
}

// LiteLLM raises these before it looks at the stream flag or the dict branch, so they win over both
fn content(text: &str) -> Result<ChatAnswer<'_>, MockError> {
    match text {
        "litellm.RateLimitError" => Err(MockError::RateLimit),
        "litellm.ContextWindowExceededError" => Err(MockError::ContextWindowExceeded),
        "litellm.InternalServerError" => Err(MockError::InternalServerError),
        _ if text.starts_with("Exception: content_filter_policy") => Err(MockError::ContentFilter),
        _ if text.starts_with("Exception: mock_streaming_error") => Err(MockError::Streaming),
        _ => Ok(ChatAnswer::Content(text)),
    }
}

pub fn response(
    plan: &MockPlan<'_>,
    usage: UsageDefaults,
    model: &str,
    id: &str,
    created: u64,
) -> Result<ChatResponse, MockError> {
    match answer(plan.response)? {
        ChatAnswer::Content(content) => Ok(ChatResponse {
            id: id.to_owned(),
            object: String::from("chat.completion"),
            created,
            model: model.to_owned(),
            choices: content_choices(content, plan)?,
            usage: usage_from(plan, usage),
            system_fingerprint: None,
            service_tier: None,
        }),
        ChatAnswer::Object(object) => Ok(object_response(object, model, id, created)),
    }
}

fn content_choices(content: &str, plan: &MockPlan<'_>) -> Result<Vec<ChatChoice>, MockError> {
    let mut choices = (0..plan.choice_count.unwrap_or(1))
        .map(|index| ChatChoice {
            index,
            message: AssistantMessage {
                role: String::from("assistant"),
                content: Some(content.to_owned()),
                tool_calls: None,
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: Some(String::from("stop")),
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        })
        .collect::<Vec<_>>();
    // LiteLLM attaches mock_tool_calls to choices[0] only, even when n asks for more
    if let Some(tool_calls) = plan.tool_calls {
        let first = choices
            .first_mut()
            .ok_or(MockError::ToolCallsWithoutChoice)?;
        first.message.tool_calls = Some(
            tool_calls
                .iter()
                .map(|call| ToolCall {
                    id: call.id.clone(),
                    kind: call.kind.clone(),
                    function: FunctionCall {
                        name: call.function.name.clone(),
                        arguments: call.function.arguments.clone(),
                    },
                })
                .collect(),
        );
    }
    Ok(choices)
}

fn usage_from(plan: &MockPlan<'_>, defaults: UsageDefaults) -> ChatUsage {
    let prompt_tokens = plan.prompt_tokens.unwrap_or(defaults.prompt_tokens);
    let completion_tokens = defaults.completion_tokens;
    ChatUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens.saturating_add(completion_tokens),
        prompt_tokens_details: None,
        completion_tokens_details: None,
    }
}

fn object_response(object: &MockChatObject, model: &str, id: &str, created: u64) -> ChatResponse {
    ChatResponse {
        id: object.id.clone().unwrap_or_else(|| id.to_owned()),
        object: String::from("chat.completion"),
        created: object.created.unwrap_or(created),
        model: object.model.clone().unwrap_or_else(|| model.to_owned()),
        choices: object_choices(object),
        usage: object
            .usage
            .as_ref()
            .map_or_else(ChatUsage::default, |usage| ChatUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
                prompt_tokens_details: None,
                completion_tokens_details: None,
            }),
        system_fingerprint: object.system_fingerprint.clone(),
        service_tier: object.service_tier.clone(),
    }
}

fn object_choices(object: &MockChatObject) -> Vec<ChatChoice> {
    object
        .choices
        .clone()
        .unwrap_or_else(|| vec![MockChatChoice::default()])
        .into_iter()
        .map(|choice| ChatChoice {
            index: choice.index,
            message: AssistantMessage {
                role: choice.message.role,
                content: choice.message.content,
                tool_calls: choice.message.tool_calls.map(|calls| {
                    calls
                        .into_iter()
                        .map(|call| ToolCall {
                            id: call.id,
                            kind: call.kind,
                            function: FunctionCall {
                                name: call.function.name,
                                arguments: call.function.arguments,
                            },
                        })
                        .collect()
                }),
                annotations: None,
                audio: None,
                refusal: None,
                provider_specific_fields: MessageProviderFields::default(),
            },
            finish_reason: choice.finish_reason,
            logprobs: None,
            provider_specific_fields: BTreeMap::new(),
        })
        .collect()
}

mod tests;
