use betterlite_config::sections::mock_response::failure::MockError;
use betterlite_config::sections::mock_response::selection::MockPlan;
use betterlite_config::sections::mock_response::{
    MockChatChoice, MockChatMessage, MockChatObject, MockResponse, MockToolCall, MockUsage,
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
        Some(MockResponse::Text(text)) => match text.as_str() {
            "litellm.RateLimitError" => return Err(MockError::RateLimit),
            "litellm.ContextWindowExceededError" => return Err(MockError::ContextWindowExceeded),
            "litellm.InternalServerError" => return Err(MockError::InternalServerError),
            _ if text.starts_with("Exception: content_filter_policy") => {
                return Err(MockError::ContentFilter);
            }
            _ if text.starts_with("Exception: mock_streaming_error") => {
                return Err(MockError::Streaming);
            }
            _ => ChatAnswer::Content(text),
        },
        Some(MockResponse::Object(object)) => ChatAnswer::Object(object),
        Some(MockResponse::Numbers(_)) => {
            return Err(MockError::WrongType {
                field: "mock_response",
            });
        }
    };
    if let Some(count) = plan.choice_count.filter(|count| *count > MAX_MOCK_CHOICES) {
        return Err(MockError::TooManyChoices {
            count,
            limit: MAX_MOCK_CHOICES,
        });
    }
    Ok(answer)
}

pub fn response(
    plan: &MockPlan<'_>,
    usage: UsageDefaults,
    model: &str,
    id: &str,
    created: u64,
) -> Result<ChatResponse, MockError> {
    match answer(plan)? {
        ChatAnswer::Content(content) => {
            let prompt_tokens = plan.prompt_tokens.unwrap_or(usage.prompt_tokens);
            Ok(object_response(
                &MockChatObject {
                    id: Some(id.to_owned()),
                    created: Some(created),
                    model: Some(model.to_owned()),
                    object: None,
                    system_fingerprint: None,
                    service_tier: None,
                    choices: Some(content_choices(content, plan)?),
                    usage: Some(MockUsage {
                        prompt_tokens,
                        completion_tokens: usage.completion_tokens,
                        total_tokens: prompt_tokens.saturating_add(usage.completion_tokens),
                    }),
                },
                model,
                id,
                created,
            ))
        }
        ChatAnswer::Object(object) => Ok(object_response(object, model, id, created)),
    }
}

fn content_choices(content: &str, plan: &MockPlan<'_>) -> Result<Vec<MockChatChoice>, MockError> {
    let mut choices = (0..plan.choice_count.unwrap_or(1))
        .map(|index| MockChatChoice {
            index,
            message: MockChatMessage {
                role: String::from("assistant"),
                content: Some(content.to_owned()),
                tool_calls: None,
            },
            finish_reason: Some(String::from("stop")),
        })
        .collect::<Vec<_>>();
    // LiteLLM attaches mock_tool_calls to choices[0] only, even when n asks for more
    if let Some(tool_calls) = plan.tool_calls {
        let first = choices
            .first_mut()
            .ok_or(MockError::ToolCallsWithoutChoice)?;
        first.message.tool_calls = Some(tool_calls.to_vec());
    }
    Ok(choices)
}

// ChatCompletionMessageToolCall fills a uuid id and the function type, and a missing argument list is empty
fn wire_tool_call(call: &MockToolCall) -> ToolCall {
    let id = call
        .id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let kind = call
        .kind
        .clone()
        .unwrap_or_else(|| String::from("function"));
    ToolCall {
        id,
        kind,
        function: FunctionCall {
            name: call.function.name.clone(),
            arguments: call.function.arguments.clone().unwrap_or_default(),
        },
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
