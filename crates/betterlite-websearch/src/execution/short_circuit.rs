use betterlite_anthropic::feature_flags::tools::is_native_web_search_tool;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::Message;
use uuid::Uuid;

use super::response::{
    AssistantRole, MessageKind, SearchUsage, ServerToolUseBlock, StopReason, SyntheticContentBlock,
    SyntheticSearchResponse, TextBlock, TextKind,
};
use crate::call_transformation::native::outcome_block;
use crate::call_transformation::outcome::search_outcome_text;
use crate::records::{AnthropicSearchQuery, SearchOutcome, ServerToolUseType};
use crate::tools::Tool;

const WEB_SEARCH_TOOL_NAME: &str = "web_search";

pub struct ShortCircuitRequest<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub tools: Option<&'request [Tool]>,
    pub custom_llm_provider: Option<&'request str>,
}

#[must_use]
pub fn anthropic_native_tool(tool: &Tool) -> bool {
    let Some(kind) = &tool.tool_type else {
        return false;
    };
    // B568's detector reads only the tool's type field
    let mut fields = Object::new();
    fields.insert("type".to_owned(), Json::Text(kind.clone()));
    is_native_web_search_tool(&Json::Object(fields))
}

#[must_use]
pub fn synthetic_response(
    model: &str,
    query: &str,
    native_tool: Option<&Tool>,
    outcome: &SearchOutcome,
) -> SyntheticSearchResponse {
    let mut content = Vec::new();
    if let Some(tool) = native_tool {
        let tool_use_id = format!("srvtoolu_{}", Uuid::new_v4().simple());
        content.push(SyntheticContentBlock::ServerToolUse(ServerToolUseBlock {
            block_kind: ServerToolUseType::ServerToolUse,
            id: tool_use_id.clone(),
            name: tool
                .name
                .clone()
                .unwrap_or_else(|| WEB_SEARCH_TOOL_NAME.to_owned()),
            input: AnthropicSearchQuery {
                query: query.to_owned(),
            },
        }));
        content.push(SyntheticContentBlock::ToolResult(outcome_block(
            &tool_use_id,
            outcome,
        )));
    }
    content.push(SyntheticContentBlock::Text(TextBlock {
        block_kind: TextKind::Text,
        text: search_outcome_text(outcome),
    }));
    SyntheticSearchResponse {
        id: format!("msg_{}", Uuid::new_v4()),
        message_kind: MessageKind::Message,
        role: AssistantRole::Assistant,
        model: model.to_owned(),
        content,
        stop_reason: StopReason::EndTurn,
        stop_sequence: None,
        usage: SearchUsage {
            input_tokens: 0,
            output_tokens: 0,
        },
    }
}
