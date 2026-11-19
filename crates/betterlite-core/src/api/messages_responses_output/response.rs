use betterlite_models::decisions::Json as WireJson;
use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::messages_chat_output::output::{
    AnthropicStopReason, AssistantRole, ContentBlock, ContentEntry, MessageKind, MessageResponse,
};
use crate::api::messages_options::refusal::{
    OutputItem, refusal_stop_details, responses_output_refusal_text,
};
use crate::api::messages_responses_input::thinking::{SummaryPart, ThinkingBlock, thinking_block};
use crate::api::messages_responses_input::usage::anthropic_usage;

use super::decode_messages_responses_out;
use super::error::MessagesResponsesOutputError;
use super::records::ResponseRecord;

type WireFields = IndexMap<String, WireJson>;

pub fn translate_response(
    response: &ResponseRecord,
    limits: &ApiLimits,
) -> Result<MessageResponse, MessagesResponsesOutputError> {
    let wire = output_wire(&response.output)?;
    let refused: Vec<OutputItem<'_>> = match_refusal_items(&response.output, &wire);
    let refusal = responses_output_refusal_text(&refused);
    let mut translation = Translation {
        content: Vec::new(),
        stop_reason: AnthropicStopReason::EndTurn,
        limits,
    };
    for raw in &wire {
        translation.append(raw)?;
    }
    let stop_reason = final_stop_reason(response, refusal, translation.stop_reason);
    let stop_details = (stop_reason == AnthropicStopReason::Refusal)
        .then(|| refusal_stop_details(refusal.map(ToOwned::to_owned)));
    Ok(MessageResponse {
        id: response.id.clone(),
        kind: MessageKind::Message,
        role: AssistantRole::Assistant,
        model: response
            .model
            .clone()
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| "unknown-model".to_owned()),
        stop_sequence: None,
        usage: anthropic_usage(response.usage.as_ref())
            .map_err(|source| MessagesResponsesOutputError::Usage { source })?,
        content: translation.content,
        stop_reason,
        stop_details,
        context_management: None,
    })
}

fn match_refusal_items<'item>(
    output: &'item [ResponseOutputItem],
    wire: &'item [WireJson],
) -> Vec<OutputItem<'item>> {
    output
        .iter()
        .zip(wire)
        .map(|(item, raw)| {
            let ResponseOutputItem::Sdk(sdk) = item else {
                return OutputItem::Raw(raw);
            };
            OutputItem::Typed(sdk.as_ref())
        })
        .collect()
}

fn final_stop_reason(
    response: &ResponseRecord,
    refusal: Option<&str>,
    fallback: AnthropicStopReason,
) -> AnthropicStopReason {
    if response.status.as_deref() == Some("incomplete") {
        return AnthropicStopReason::MaxTokens;
    }
    if refusal.is_some() {
        return AnthropicStopReason::Refusal;
    }
    fallback
}

fn output_wire(
    output: &[ResponseOutputItem],
) -> Result<Vec<WireJson>, MessagesResponsesOutputError> {
    let bytes = serde_json::to_vec(output)
        .map_err(|source| MessagesResponsesOutputError::Output { source })?;
    serde_json::from_slice(&bytes).map_err(|source| MessagesResponsesOutputError::Output { source })
}

struct Translation<'limits> {
    content: Vec<ContentEntry>,
    stop_reason: AnthropicStopReason,
    limits: &'limits ApiLimits,
}

impl Translation<'_> {
    fn append(&mut self, raw: &WireJson) -> Result<(), MessagesResponsesOutputError> {
        let WireJson::Object(fields) = raw else {
            return Ok(());
        };
        match field_text(fields.get("type")) {
            Some("message") => self.append_message(fields),
            Some("reasoning") => self.append_reasoning(fields),
            Some("function_call") => return self.append_function_call(fields),
            _ => {}
        }
        Ok(())
    }

    fn append_message(&mut self, fields: &WireFields) {
        let Some(WireJson::List(parts)) = fields.get("content") else {
            return;
        };
        for part in parts {
            let WireJson::Object(fields) = part else {
                continue;
            };
            match field_text(fields.get("type")) {
                Some("output_text") => self.push_text(field_text(fields.get("text"))),
                Some("refusal") => self.push_text(field_text(fields.get("refusal"))),
                _ => {}
            }
        }
    }

    fn append_reasoning(&mut self, fields: &WireFields) {
        let mut summary: Vec<SummaryPart> = Vec::new();
        if let Some(WireJson::List(parts)) = fields.get("summary") {
            summary.extend(parts.iter().map(|part| SummaryPart {
                text: summary_text(part).to_owned(),
            }));
        }
        let encrypted = field_text(fields.get("encrypted_content"));
        self.push_thinking(thinking_block(&summary, encrypted));
    }

    fn append_function_call(
        &mut self,
        fields: &WireFields,
    ) -> Result<(), MessagesResponsesOutputError> {
        let input = match fields.get("arguments") {
            Some(WireJson::Text(arguments)) => arguments_input(arguments, self.limits)?,
            _ => Object::new(),
        };
        let id = field_text(fields.get("call_id"))
            .filter(|call_id| !call_id.is_empty())
            .or_else(|| field_text(fields.get("id")))
            .unwrap_or("");
        let name = field_text(fields.get("name")).unwrap_or("");
        self.append_tool_use(id, name, input);
        Ok(())
    }

    fn append_tool_use(&mut self, id: &str, name: &str, input: Object) {
        self.content
            .push(ContentEntry::Block(ContentBlock::ToolUse {
                id: id.to_owned(),
                name: name.to_owned(),
                input: Json::Object(input),
                provider_specific_fields: None,
            }));
        self.stop_reason = AnthropicStopReason::ToolUse;
    }

    fn push_text(&mut self, text: Option<&str>) {
        self.content.push(ContentEntry::Block(ContentBlock::Text {
            text: text.unwrap_or("").to_owned(),
        }));
    }

    fn push_thinking(&mut self, block: Option<ThinkingBlock>) {
        match block {
            Some(ThinkingBlock::Thinking {
                thinking,
                signature,
            }) => self
                .content
                .push(ContentEntry::Block(ContentBlock::Thinking {
                    thinking,
                    signature,
                })),
            Some(ThinkingBlock::RedactedThinking { data }) => {
                self.content
                    .push(ContentEntry::Block(ContentBlock::RedactedThinking { data }));
            }
            None => {}
        }
    }
}

fn arguments_input(
    arguments: &str,
    limits: &ApiLimits,
) -> Result<Object, MessagesResponsesOutputError> {
    if arguments.is_empty() {
        return Ok(Object::new());
    }
    match decode_messages_responses_out(arguments.as_bytes(), limits) {
        Ok(Json::Object(fields)) => Ok(fields),
        Ok(_) => Err(MessagesResponsesOutputError::ArgumentsNotObject),
        Err(MessagesResponsesOutputError::Json { .. }) => Ok(Object::new()),
        Err(error) => Err(error),
    }
}

const fn field_text(value: Option<&WireJson>) -> Option<&str> {
    match value {
        Some(WireJson::Text(text)) => Some(text.as_str()),
        Some(
            WireJson::Null
            | WireJson::Bool(_)
            | WireJson::Number(_)
            | WireJson::List(_)
            | WireJson::Object(_),
        )
        | None => None,
    }
}

fn summary_text(part: &WireJson) -> &str {
    if let WireJson::Object(fields) = part {
        return field_text(fields.get("text")).unwrap_or("");
    }
    ""
}
