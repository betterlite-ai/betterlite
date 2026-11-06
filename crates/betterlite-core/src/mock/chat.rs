use std::collections::BTreeMap;

use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::MockPlan;
use betterlite_config::sections::mock_response::{
    MockArguments, MockChatChoice, MockChatObject, MockFunctionCall, MockResponse, MockToolCall,
    UsageDefaults,
};
use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_openai::chat::{FunctionCall, ToolCall, choice_finish_reason};
use uuid::Uuid;

// LiteLLM's mock_completion answer when the caller and the operator both leave mock_response unset
pub const DEFAULT_CONTENT: &str = "This is a mock request";
// LiteLLM builds one choice per n before it answers, so a mock caps n instead of allocating billions
pub const MAX_MOCK_CHOICES: u32 = 128;

pub enum ChatAnswer<'content> {
    Content(&'content str),
    Object(&'content MockChatObject),
}

// LiteLLM raises a sentinel before it looks at n, and builds every choice in memory after that
pub fn answer<'plan>(plan: &'plan MockPlan<'plan>) -> Result<ChatAnswer<'plan>, MockError> {
    let answer = match plan.response {
        None => ChatAnswer::Content(DEFAULT_CONTENT),
        Some(MockResponse::Text(text)) => content(text)?,
        Some(MockResponse::Object(object)) => ChatAnswer::Object(object),
        Some(MockResponse::Numbers(_)) => {
            return Err(MockError::WrongType {
                field: "mock_response",
            });
        }
    };
    checked_choice_count(plan)?;
    Ok(answer)
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
    match answer(plan)? {
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

const fn checked_choice_count(plan: &MockPlan<'_>) -> Result<Option<u32>, MockError> {
    match plan.choice_count {
        Some(count) if count > MAX_MOCK_CHOICES => Err(MockError::TooManyChoices {
            count,
            limit: MAX_MOCK_CHOICES,
        }),
        count => Ok(count),
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
        first.message.tool_calls = Some(tool_calls.iter().map(wire_tool_call).collect());
    }
    Ok(choices)
}

// ChatCompletionMessageToolCall fills a uuid id and the function type, and Function encodes object arguments as JSON
fn wire_tool_call(call: &MockToolCall) -> ToolCall {
    ToolCall {
        id: call
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        kind: call
            .kind
            .clone()
            .unwrap_or_else(|| String::from("function")),
        function: FunctionCall {
            name: call.function.name.clone(),
            arguments: tool_arguments(&call.function),
        },
    }
}

fn tool_arguments(function: &MockFunctionCall) -> String {
    match &function.arguments {
        None => String::new(),
        Some(MockArguments::Text(text)) => text.clone(),
        Some(MockArguments::Object(value)) => value.to_string(),
    }
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
    // an object field wins over the id, timestamp and model LiteLLM generates for a mock
    let identifier = object.id.clone().unwrap_or_else(|| id.to_owned());
    let timestamp = object.created.unwrap_or(created);
    let name = object.model.clone().unwrap_or_else(|| model.to_owned());
    ChatResponse {
        id: identifier,
        object: String::from("chat.completion"),
        created: timestamp,
        model: name,
        choices: object_choices(object),
        usage: object_usage(object),
        system_fingerprint: object.system_fingerprint.clone(),
        service_tier: object.service_tier.clone(),
    }
}

fn object_usage(object: &MockChatObject) -> ChatUsage {
    object
        .usage
        .as_ref()
        .map_or_else(ChatUsage::default, |usage| ChatUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        })
}

fn object_choices(object: &MockChatObject) -> Vec<ChatChoice> {
    object
        .choices
        .clone()
        .unwrap_or_else(|| vec![MockChatChoice::default()])
        .into_iter()
        .map(|choice| {
            let (finish_reason, native) = choice_finish_reason(choice.finish_reason.as_deref());
            ChatChoice {
                index: choice.index,
                message: AssistantMessage {
                    role: choice.message.role,
                    content: choice.message.content,
                    tool_calls: choice
                        .message
                        .tool_calls
                        .as_deref()
                        .map(|calls| calls.iter().map(wire_tool_call).collect()),
                    annotations: None,
                    audio: None,
                    refusal: None,
                    provider_specific_fields: MessageProviderFields::default(),
                },
                finish_reason: Some(finish_reason),
                logprobs: None,
                provider_specific_fields: native,
            }
        })
        .collect()
}

mod tests;
