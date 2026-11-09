use std::collections::BTreeMap;

use betterlite_openai::chat::response::{
    AssistantMessage, ChatChoice, ChatResponse, ChatUsage, MessageProviderFields,
};
use betterlite_openai::chat::{FunctionCall as OpenAiChatFunctionCall, ToolCall};
use rust_decimal::Decimal;

use crate::sampling_convert::content::{
    McpContentBlock, SamplingContent, SamplingRole, ToolResultBlock,
};
use crate::sampling_convert::json_object::JsonObject;
use crate::sampling_convert::messages::SamplingMessage;
use crate::sampling_convert::model_preferences::{ModelHint, ModelInfo, ModelPreferences};
use crate::sampling_convert::openai_wire::{
    OpenAiContent, OpenAiContentPart, OpenAiFunctionCall, OpenAiMessage, OpenAiRole,
    OpenAiToolCall, ToolKind,
};

pub(crate) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("a JSON object")
}

pub(crate) fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn hint(name: Option<&str>) -> ModelHint {
    ModelHint {
        name: name.unwrap_or_default().to_owned(),
    }
}

pub(crate) fn with_priorities(
    cost: Option<Decimal>,
    speed: Option<Decimal>,
    intelligence: Option<Decimal>,
) -> ModelPreferences {
    ModelPreferences {
        hints: Vec::new(),
        cost_priority: cost.unwrap_or_default(),
        speed_priority: speed.unwrap_or_default(),
        intelligence_priority: intelligence.unwrap_or_default(),
    }
}

pub(crate) fn metrics(cost: i64, max_output: u64, tps: i64) -> ModelInfo {
    ModelInfo {
        input_cost_per_token: Some(Decimal::new(cost, 6)),
        output_cost_per_token: Some(Decimal::new(cost, 6)),
        max_output_tokens: Some(max_output),
        max_tokens: None,
        output_tokens_per_second: Some(Decimal::from(tps)),
    }
}

pub(crate) fn scored_catalog() -> BTreeMap<String, ModelInfo> {
    BTreeMap::from([
        (String::from("cheap"), metrics(1, 1_000, 10)),
        (String::from("fast"), metrics(10, 4_000, 100)),
        (String::from("smart"), metrics(100, 64_000, 1)),
    ])
}

pub(crate) fn message(role: SamplingRole, content: SamplingContent) -> SamplingMessage {
    SamplingMessage { role, content }
}

pub(crate) fn text(value: &str) -> McpContentBlock {
    McpContentBlock::Text {
        text: value.to_owned(),
    }
}

pub(crate) fn tool_use(id: &str, name: &str, input: &str) -> McpContentBlock {
    McpContentBlock::ToolUse {
        id: id.to_owned(),
        name: name.to_owned(),
        input: object(input),
    }
}

pub(crate) fn tool_result(id: &str, content: Vec<ToolResultBlock>) -> McpContentBlock {
    McpContentBlock::ToolResult {
        tool_use_id: id.to_owned(),
        content,
    }
}

pub(crate) fn text_block(value: &str) -> ToolResultBlock {
    ToolResultBlock::Text {
        text: value.to_owned(),
    }
}

pub(crate) fn single(block: McpContentBlock) -> SamplingContent {
    SamplingContent::Single(block)
}

pub(crate) fn many(blocks: Vec<McpContentBlock>) -> SamplingContent {
    SamplingContent::Many(blocks)
}

pub(crate) fn text_parts_message(value: &str) -> OpenAiMessage {
    OpenAiMessage {
        role: OpenAiRole::User,
        content: Some(OpenAiContent::Parts(vec![OpenAiContentPart::Text {
            text: value.to_owned(),
        }])),
        tool_calls: None,
        tool_call_id: None,
    }
}

pub(crate) fn expected_call(id: &str, name: &str, arguments: &str) -> OpenAiToolCall {
    OpenAiToolCall {
        id: id.to_owned(),
        kind: ToolKind::Function,
        function: OpenAiFunctionCall {
            name: name.to_owned(),
            arguments: arguments.to_owned(),
        },
    }
}

pub(crate) fn choice(
    content: Option<&str>,
    calls: Option<&[(&str, &str, &str)]>,
    finish_reason: Option<&str>,
) -> ChatChoice {
    let tool_calls = calls.map(|given| {
        given
            .iter()
            .map(|(id, name, arguments)| ToolCall {
                id: (*id).to_owned(),
                kind: String::from("function"),
                function: OpenAiChatFunctionCall {
                    name: (*name).to_owned(),
                    arguments: (*arguments).to_owned(),
                },
            })
            .collect()
    });
    ChatChoice {
        index: 0,
        message: AssistantMessage {
            role: String::from("assistant"),
            content: content.map(str::to_owned),
            tool_calls,
            annotations: None,
            audio: None,
            refusal: None,
            provider_specific_fields: MessageProviderFields::default(),
        },
        finish_reason: finish_reason.map(str::to_owned),
        logprobs: None,
        provider_specific_fields: BTreeMap::new(),
    }
}

pub(crate) fn response(model: &str, choices: Vec<ChatChoice>) -> ChatResponse {
    ChatResponse {
        id: String::from("chatcmpl-1"),
        object: String::from("chat.completion"),
        created: 1,
        model: model.to_owned(),
        choices,
        usage: ChatUsage::default(),
        system_fingerprint: None,
        service_tier: None,
    }
}
