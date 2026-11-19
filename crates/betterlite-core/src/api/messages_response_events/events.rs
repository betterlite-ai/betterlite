use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::sdk::unions::SdkOutput;
use betterlite_prompts::reasoning_fields::encrypted_signature::encrypted_reasoning_signature;

use crate::api::messages_chat_output::output::AnthropicStopReason;
use crate::api::messages_options::refusal::{OutputItem, responses_output_refusal_text};
use crate::api::messages_responses_input::usage::anthropic_usage;

use super::MessageStreamState;
use super::REASONING_SUMMARY_PART_SEPARATOR;
use super::chunks;
use super::error::MessagesResponseEventError;
use super::input::{ItemObject, ParsedEvent, ResponseAttribute, is_truthy, text};

const NO_OUTPUT: &[OutputItem<'static>] = &[];

impl MessageStreamState {
    // _process_event: one parsed Responses event becomes zero or more queued Anthropic chunks.
    pub fn process_event(
        &mut self,
        event: ParsedEvent<'_>,
    ) -> Result<(), MessagesResponseEventError> {
        let Some(event_type) = event.event_type() else {
            return Ok(());
        };
        match event_type {
            "response.created" => self.start_message(),
            "response.output_item.added" => self.item_added(event),
            "response.refusal.delta" => self.refusal_delta(event),
            "response.output_text.delta" => self.text_delta(event),
            "response.reasoning_summary_part.added" => self.summary_part_added(event),
            "response.reasoning_summary_text.delta" => self.summary_text_delta(event),
            "response.function_call_arguments.delta" => self.arguments_delta(event),
            "response.output_item.done" => self.item_done(event),
            "response.failed" => return self.failed(event),
            "response.completed" | "response.incomplete" => return self.completed(event),
            _ => {}
        }
        Ok(())
    }

    fn item_added(&mut self, event: ParsedEvent<'_>) {
        let Some(item) = event.item() else {
            return;
        };
        match item.truthy_field("type").and_then(text) {
            Some("message") => {
                let item_id = item.truthy_field("id");
                self.open_block(item_id, chunks::text_block());
            }
            Some("function_call") => self.open_function_call(item),
            Some(_) | None => {}
        }
    }

    fn open_function_call(&mut self, item: ItemObject<'_>) {
        let call_id = item
            .truthy_field("call_id")
            .cloned()
            .unwrap_or(Json::Text(String::new()));
        let name = item
            .truthy_field("name")
            .cloned()
            .unwrap_or(Json::Text(String::new()));
        let item_id = item.truthy_field("id");
        if let Some(Json::Text(text_id)) = item_id {
            self.pending_tool_ids
                .insert(text_id.clone(), call_id.clone());
        }
        self.open_block(item_id, chunks::tool_block(&call_id, &name));
    }

    fn refusal_delta(&mut self, event: ParsedEvent<'_>) {
        let Some(delta) = event.delta().and_then(text) else {
            return;
        };
        if delta.is_empty() {
            return;
        }
        self.refusal_text.push_str(delta);
        let item_id = event.item_id();
        let mut block_idx = self.block_index_for(item_id);
        if block_idx < 0 {
            block_idx = self.open_block(item_id, chunks::text_block());
        }
        self.push_delta(block_idx, chunks::text_delta(Json::Text(delta.to_owned())));
    }

    fn text_delta(&mut self, event: ParsedEvent<'_>) {
        let item_id = event.item_id();
        let mut block_idx = self.block_index_for(item_id);
        if block_idx < 0 {
            block_idx = self.open_block(item_id, chunks::text_block());
        }
        let delta = chunks::text_delta(event.delta().cloned().unwrap_or(Json::Null));
        self.push_delta(block_idx, delta);
    }

    fn summary_part_added(&mut self, event: ParsedEvent<'_>) {
        let part_block_idx = self.mapped_block_index(event.field("item_id"));
        if part_block_idx < 0 || !qualifies_as_summary_index(event.field("summary_index")) {
            return;
        }
        let separator = Json::Text(REASONING_SUMMARY_PART_SEPARATOR.to_owned());
        self.push_delta(part_block_idx, chunks::thinking_delta(separator));
    }

    fn summary_text_delta(&mut self, event: ParsedEvent<'_>) {
        let item_id = event.item_id();
        let delta = event.delta();
        let mut block_idx = self.block_index_for(item_id);
        if block_idx < 0 {
            if !delta.is_some_and(is_truthy) {
                return;
            }
            block_idx = self.open_block(item_id, chunks::thinking_block());
        }
        self.push_delta(
            block_idx,
            chunks::thinking_delta(delta.cloned().unwrap_or(Json::Null)),
        );
    }

    fn arguments_delta(&mut self, event: ParsedEvent<'_>) {
        let block_idx = match event.item_id().filter(|value| is_truthy(value)) {
            Some(Json::Text(item_id)) => self
                .item_id_to_block_index
                .get(item_id)
                .copied()
                .unwrap_or(self.current_block_index),
            Some(_) | None => self.current_block_index,
        };
        let delta = chunks::input_json_delta(event.delta().cloned().unwrap_or(Json::Null));
        self.push_delta(block_idx, delta);
    }

    fn item_done(&mut self, event: ParsedEvent<'_>) {
        let item = event.item();
        let item_id = item.and_then(|found| found.truthy_field("id"));
        if item.is_some_and(|found| found.field("type").and_then(text) == Some("reasoning")) {
            self.close_reasoning_item(item, item_id);
            return;
        }
        let block_idx = self.block_index_for(item_id);
        if block_idx < 0 {
            return;
        }
        self.chunk_queue
            .push_back(chunks::content_block_stop(block_idx));
    }

    fn close_reasoning_item(&mut self, item: Option<ItemObject<'_>>, item_id: Option<&Json>) {
        let block_idx = self.block_index_for(item_id);
        let signature = item
            .and_then(|item| item.field("encrypted_content"))
            .and_then(text)
            .filter(|text| !text.is_empty())
            .map(encrypted_reasoning_signature);
        if block_idx < 0 && signature.is_none() {
            return;
        }
        if block_idx < 0 {
            let data = signature.unwrap_or_default();
            let redacted_idx = self.open_block(item_id, chunks::redacted_thinking_block(&data));
            self.chunk_queue
                .push_back(chunks::content_block_stop(redacted_idx));
            return;
        }
        if let Some(encoded) = signature {
            self.push_delta(block_idx, chunks::signature_delta(&encoded));
        }
        self.chunk_queue
            .push_back(chunks::content_block_stop(block_idx));
    }

    fn completed(&mut self, event: ParsedEvent<'_>) -> Result<(), MessagesResponseEventError> {
        let (output, status, usage) = match event.response() {
            Some(ResponseAttribute::Object(attributes)) => {
                (attributes.output, attributes.status, attributes.usage)
            }
            Some(ResponseAttribute::Value(_)) | None => (NO_OUTPUT, None, None),
        };
        let refusal_text = final_refusal_text(output, &self.refusal_text);
        let stop_reason = stop_reason(status, refusal_text.as_deref(), output);
        let projected_usage = anthropic_usage(usage).map_err(MessagesResponseEventError::Usage)?;
        let stop_details = refusal_text
            .as_deref()
            .filter(|_| stop_reason == AnthropicStopReason::Refusal);
        let delta = chunks::message_delta(stop_reason, stop_details, &projected_usage)?;
        self.chunk_queue.push_back(delta);
        self.chunk_queue.push_back(chunks::message_stop());
        self.sent_message_stop = true;
        Ok(())
    }

    fn push_delta(&mut self, block_idx: i64, delta: Json) {
        self.chunk_queue
            .push_back(chunks::content_block_delta(block_idx, delta));
    }
}

fn final_refusal_text(output: &[OutputItem<'_>], streamed: &str) -> Option<String> {
    match responses_output_refusal_text(output) {
        Some(text) => Some(text.to_owned()),
        None if streamed.is_empty() => None,
        None => Some(streamed.to_owned()),
    }
}

fn stop_reason(
    status: Option<&str>,
    refusal_text: Option<&str>,
    output: &[OutputItem<'_>],
) -> AnthropicStopReason {
    if status == Some("incomplete") {
        return AnthropicStopReason::MaxTokens;
    }
    if refusal_text.is_some() {
        return AnthropicStopReason::Refusal;
    }
    if output.iter().any(is_function_call) {
        return AnthropicStopReason::ToolUse;
    }
    AnthropicStopReason::EndTurn
}

fn is_function_call(item: &OutputItem<'_>) -> bool {
    match item {
        OutputItem::Typed(SdkOutput::FunctionCall(_)) => true,
        OutputItem::Typed(_) => false,
        OutputItem::Raw(raw) => matches!(
            raw,
            Json::Object(fields)
                if matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "function_call")
        ),
    }
}

// isinstance(summary_index, int) includes bools; a float or a non-integer string never qualifies.
fn qualifies_as_summary_index(value: Option<&Json>) -> bool {
    match value {
        Some(Json::Bool(flag)) => *flag,
        Some(Json::Number(number)) => {
            let text = number.to_string();
            !text.contains(['.', 'e', 'E']) && text != "0" && text != "-0"
        }
        Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) | None => false,
    }
}
