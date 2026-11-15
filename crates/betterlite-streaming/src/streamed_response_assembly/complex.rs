use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};
use crate::stream_content_assembly::wire::{AudioResponse, ThinkingBlock};
use crate::stream_content_assembly::{
    ContentField, get_combined_audio_content, get_combined_content, get_combined_thinking_content,
};
use crate::stream_tool_assembly::assemble;
use crate::stream_tool_assembly::records::{
    AssemblyInput, AssemblyOutput, FunctionInput, FunctionOutput, ToolInput, ToolOutput,
};

use super::content::combined_provider_fields;
use super::records::{Choice, Chunk, Delta};
use super::response::Draft;
use super::{AssemblyError, project};

// The non-simple path: each field joins its own selected chunks, and later content overwrites a cleared text.
pub(crate) fn apply(draft: &mut Draft, chunks: &[Chunk], now: i64) -> Result<(), AssemblyError> {
    if let Some(calls) = joined_tool_calls(chunks)? {
        clear_content(draft);
        draft.extras.tool_calls = Some(calls);
    }
    if let Some(call) = joined_function_call(chunks)? {
        clear_content(draft);
        draft.extras.function_call = Some(call);
    }
    if let Some(content) = joined_content(chunks, ContentField::Content)? {
        set_content(draft, content);
    }
    draft.extras.thinking_blocks = joined_thinking(chunks)?;
    if let Some(reasoning) = joined_content(chunks, ContentField::Reasoning)? {
        draft.extras.reasoning_content = Some(reasoning);
    }
    draft.extras.annotations = joined_lists(chunks, ListField::Annotations);
    if let Some(audio) = joined_audio(chunks, now)? {
        draft.extras.audio = Some(audio);
    }
    draft.extras.images = joined_lists(chunks, ListField::Images);
    draft.extras.provider_specific_fields = joined_provider_fields(chunks);
    Ok(())
}

// LiteLLM assigns through Choices, which owns the one message of the base choice.
fn clear_content(draft: &mut Draft) {
    if let Some(choice) = draft.base.choices.first_mut() {
        choice.message.content = None;
    }
}

fn set_content(draft: &mut Draft, content: String) {
    if let Some(choice) = draft.base.choices.first_mut() {
        choice.message.content = Some(content);
    }
}

fn first_delta(chunk: &Chunk) -> Option<&Delta> {
    chunk
        .choices
        .first()
        .and_then(|choice| choice.delta.as_ref())
}

fn has_tool_calls(chunk: &Chunk) -> bool {
    chunk.choices.iter().any(|choice| {
        choice
            .delta
            .as_ref()
            .and_then(|delta| delta.tool_calls.as_ref())
            .is_some()
    })
}

fn joined_tool_calls(chunks: &[Chunk]) -> Result<Option<Vec<ToolOutput>>, AssemblyError> {
    let selected: Vec<&Chunk> = chunks
        .iter()
        .filter(|chunk| has_tool_calls(chunk))
        .collect();
    if selected.is_empty() {
        return Ok(None);
    }
    let projected = project::tool_chunks(&selected)?;
    let output = assemble(&AssemblyInput::Tool {
        tool: ToolInput { chunks: projected },
    })?;
    let AssemblyOutput::Tools(calls) = output else {
        return Err(AssemblyError::WrongToolOutput);
    };
    Ok(Some(calls))
}

fn joined_function_call(chunks: &[Chunk]) -> Result<Option<FunctionOutput>, AssemblyError> {
    let selected: Vec<&Chunk> = chunks
        .iter()
        .filter(|chunk| first_delta(chunk).is_some_and(|delta| delta.function_call.is_some()))
        .collect();
    if selected.is_empty() {
        return Ok(None);
    }
    let projected = project::tool_chunks(&selected)?;
    let output = assemble(&AssemblyInput::Function {
        function: FunctionInput { chunks: projected },
    })?;
    let AssemblyOutput::Function(call) = output else {
        return Err(AssemblyError::WrongFunctionOutput);
    };
    Ok(Some(call))
}

fn joined_content(chunks: &[Chunk], field: ContentField) -> Result<Option<String>, AssemblyError> {
    let selected: Vec<&Chunk> = chunks
        .iter()
        .filter(|chunk| {
            first_delta(chunk).is_some_and(|delta| match field {
                ContentField::Content => delta.content.is_some(),
                ContentField::Reasoning => delta.reasoning_content.is_some(),
            })
        })
        .collect();
    if selected.is_empty() {
        return Ok(None);
    }
    let projected = project::content_chunks(&selected)?;
    Ok(Some(get_combined_content(&projected, field)?))
}

fn joined_thinking(chunks: &[Chunk]) -> Result<Option<Vec<ThinkingBlock>>, AssemblyError> {
    let selected: Vec<&Chunk> = chunks
        .iter()
        .filter(|chunk| first_delta(chunk).is_some_and(|delta| delta.thinking_blocks.is_some()))
        .collect();
    if selected.is_empty() {
        return Ok(None);
    }
    let projected = project::content_chunks(&selected)?;
    Ok(get_combined_thinking_content(&projected)?)
}

fn joined_audio(chunks: &[Chunk], now: i64) -> Result<Option<AudioResponse>, AssemblyError> {
    let selected: Vec<&Chunk> = chunks
        .iter()
        .filter(|chunk| first_delta(chunk).is_some_and(|delta| delta.audio.is_some()))
        .collect();
    if selected.is_empty() {
        return Ok(None);
    }
    let projected = project::content_chunks(&selected)?;
    Ok(Some(get_combined_audio_content(&projected, now)?))
}

#[derive(Clone, Copy)]
enum ListField {
    Annotations,
    Images,
}

// Every selected first-choice list is concatenated in stream order, and an empty join still assigns.
fn joined_lists(chunks: &[Chunk], field: ListField) -> Option<Vec<JsonValue>> {
    let mut joined = Vec::new();
    let mut selected = false;
    for chunk in chunks {
        let Some(delta) = first_delta(chunk) else {
            continue;
        };
        let items = match field {
            ListField::Annotations => delta.annotations.as_ref(),
            ListField::Images => delta.images.as_ref(),
        };
        if let Some(list) = items {
            selected = true;
            joined.extend(list.iter().cloned());
        }
    }
    selected.then_some(joined)
}

fn joined_provider_fields(chunks: &[Chunk]) -> Option<JsonObject> {
    let selected: Vec<&Choice> = chunks
        .iter()
        .filter(|chunk| {
            first_delta(chunk).is_some_and(|delta| delta.provider_specific_fields.is_some())
        })
        .filter_map(|chunk| chunk.choices.first())
        .collect();
    combined_provider_fields(&selected)
}
