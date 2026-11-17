use betterlite_openai::api::responses_objects::ResponseOutputItem;
use betterlite_openai::api::responses_objects::sdk::text::OutputMessage;
use betterlite_openai::api::responses_objects::sdk::unions::{
    OutputMessageContentValue, OutputTextAnnotationsValue, SdkOutput,
};
use serde_json::Value;

use super::error::{ResponseChatResultError as Error, serialized};
use super::raw::raw_message_choice;
use super::reasoning::{PendingReasoning, pending};
use super::records::{ChatChoice, ChatMessage};
use super::tools::{apply_patch_chunk, is_raw_tool_item, raw_tool_chunk, typed_function_chunk};
use crate::api::response_chat_content::annotations::{AnnotationValue, annotation_fallback};
use crate::api::responses_tool_results::records::ToolCallChunk;

pub(crate) fn output_choices(output: &[ResponseOutputItem]) -> Result<Vec<ChatChoice>, Error> {
    let mut assembly = Assembly::default();
    for item in output {
        match item {
            ResponseOutputItem::Sdk(sdk) => assembly.sdk_item(sdk)?,
            ResponseOutputItem::Message(_)
            | ResponseOutputItem::FunctionCall(_)
            | ResponseOutputItem::ImageGeneration(_)
            | ResponseOutputItem::CodeInterpreter(_)
            | ResponseOutputItem::CustomTool(_)
            | ResponseOutputItem::Unknown(_) => assembly.raw_value(serialized(item)?)?,
        }
    }
    Ok(assembly.finish())
}

#[derive(Default)]
struct Assembly {
    choices: Vec<ChatChoice>,
    tool_calls: Vec<ToolCallChunk>,
    index: usize,
    tool_index: usize,
    reasoning: Option<PendingReasoning>,
}

impl Assembly {
    fn sdk_item(&mut self, output: &SdkOutput) -> Result<(), Error> {
        match output {
            SdkOutput::Reasoning(reasoning) => {
                self.reasoning = Some(pending(reasoning));
                Ok(())
            }
            SdkOutput::OutputMessage(message) => self.message(message),
            SdkOutput::FunctionCall(call) => {
                self.tool_calls.push(typed_function_chunk(call)?);
                self.tool_index = self.tool_index.saturating_add(1);
                Ok(())
            }
            SdkOutput::OutputApplyPatch(call) => {
                self.tool_calls
                    .push(apply_patch_chunk(call, self.tool_index));
                self.tool_index = self.tool_index.saturating_add(1);
                Ok(())
            }
            SdkOutput::FileSearch(_)
            | SdkOutput::FunctionCallOutput(_)
            | SdkOutput::WebSearch(_)
            | SdkOutput::Computer(_)
            | SdkOutput::ComputerOutput(_)
            | SdkOutput::Program(_)
            | SdkOutput::ProgramOutput(_)
            | SdkOutput::SearchCall(_)
            | SdkOutput::SearchOutput(_)
            | SdkOutput::AdditionalTools(_)
            | SdkOutput::Compaction(_)
            | SdkOutput::ImageCall(_)
            | SdkOutput::CodeInterpreterCall(_)
            | SdkOutput::LocalShellCall(_)
            | SdkOutput::LocalShellCallOutput(_)
            | SdkOutput::OutputShell(_)
            | SdkOutput::ShellOutput(_)
            | SdkOutput::ApplyPatchOutput(_)
            | SdkOutput::McpCall(_)
            | SdkOutput::McpListTools(_)
            | SdkOutput::McpApprovalRequest(_)
            | SdkOutput::McpApprovalResponse(_)
            | SdkOutput::CustomCall(_)
            | SdkOutput::CustomCallOutput(_) => self.raw_value(serialized(output)?),
        }
    }

    // one choice per content block, with the pending reasoning flushed onto the first one
    fn message(&mut self, message: &OutputMessage) -> Result<(), Error> {
        for part in &message.content {
            let reasoning = self.reasoning.take();
            self.choices
                .push(content_choice(part, self.index, reasoning)?);
            self.index = self.index.saturating_add(1);
        }
        Ok(())
    }

    fn raw_value(&mut self, value: Value) -> Result<(), Error> {
        let Value::Object(fields) = value else {
            return Ok(());
        };
        if is_raw_tool_item(&fields) {
            let chunk = raw_tool_chunk(&fields, self.tool_index)?;
            self.tool_calls.push(chunk);
            self.tool_index = self.tool_index.saturating_add(1);
            return Ok(());
        }
        let Some(choice) = raw_message_choice(&fields, self.index)? else {
            return Ok(());
        };
        self.index = self.index.saturating_add(1);
        self.choices.push(choice);
        Ok(())
    }

    // every accumulated tool call goes into one trailing choice, carrying the reasoning no text flushed
    fn finish(self) -> Vec<ChatChoice> {
        let Assembly {
            mut choices,
            tool_calls,
            index,
            reasoning,
            ..
        } = self;
        if tool_calls.is_empty() {
            return choices;
        }
        let mut message = ChatMessage {
            tool_calls: Some(tool_calls),
            ..ChatMessage::default()
        };
        if let Some(pending) = reasoning {
            message.reasoning_content = Some(pending.text);
            message.reasoning_items = Some(vec![pending.item]);
        }
        choices.push(ChatChoice {
            finish_reason: "tool_calls".to_owned(),
            index,
            message,
        });
        choices
    }
}

fn content_choice(
    part: &OutputMessageContentValue,
    index: usize,
    reasoning: Option<PendingReasoning>,
) -> Result<ChatChoice, Error> {
    let (content, annotations) = match part {
        OutputMessageContentValue::OutputText(text) => {
            (text.text.clone(), typed_annotations(&text.annotations)?)
        }
        OutputMessageContentValue::OutputRefusal(_) => (String::new(), None),
    };
    let mut message = ChatMessage {
        content: Some(content),
        annotations,
        ..ChatMessage::default()
    };
    if let Some(pending) = reasoning {
        message.reasoning_content = Some(pending.text);
        message.reasoning_items = Some(vec![pending.item]);
    }
    Ok(ChatChoice {
        finish_reason: "stop".to_owned(),
        index,
        message,
    })
}

fn typed_annotations(
    annotations: &[OutputTextAnnotationsValue],
) -> Result<Option<Vec<Value>>, Error> {
    let mut values = Vec::with_capacity(annotations.len());
    for annotation in annotations {
        values.push(AnnotationValue::ModelDump(serialized(annotation)?));
    }
    Ok(keep_annotations(&values))
}

// annotations are forwarded as they arrived: the chat record filters nothing out of them
pub(super) fn keep_annotations(values: &[AnnotationValue]) -> Option<Vec<Value>> {
    let Value::Array(kept) = annotation_fallback(Some(values)).value else {
        return None;
    };
    Some(kept)
}
