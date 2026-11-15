use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};
use crate::stream_content_assembly::{
    ContentField, get_combined_audio_content, get_combined_content, get_combined_thinking_content,
};
use crate::stream_tool_assembly::assemble;
use crate::stream_tool_assembly::records::{
    AssemblyInput, AssemblyOutput, FunctionInput, ToolInput,
};

use super::content::combined_provider_fields;
use super::records::{Choice, Chunk, Delta};
use super::response::Draft;
use super::{AssemblyError, project};

// The non-simple path: each field joins its own selected chunks, and later content overwrites a cleared text.
pub(crate) fn apply(draft: &mut Draft, chunks: &[Chunk], now: i64) -> Result<(), AssemblyError> {
    apply_calls(draft, chunks)?;
    apply_content(draft, chunks, now)?;
    draft.extras.annotations = joined_lists(chunks, |delta| delta.annotations.as_deref());
    draft.extras.images = joined_lists(chunks, |delta| delta.images.as_deref());
    draft.extras.provider_specific_fields = joined_provider_fields(chunks);
    Ok(())
}

fn apply_calls(draft: &mut Draft, chunks: &[Chunk]) -> Result<(), AssemblyError> {
    let tool_chunks: Vec<&Chunk> = chunks
        .iter()
        .filter(|chunk| has_tool_calls(chunk))
        .collect();
    if !tool_chunks.is_empty() {
        let input = AssemblyInput::Tool {
            tool: ToolInput {
                chunks: project::tool_chunks(&tool_chunks)?,
            },
        };
        let AssemblyOutput::Tools(calls) = assemble(&input)? else {
            return Err(AssemblyError::WrongToolOutput);
        };
        set_content(draft, None);
        draft.extras.tool_calls = Some(calls);
    }
    let function_chunks = select(chunks, |delta| delta.function_call.is_some());
    if !function_chunks.is_empty() {
        let input = AssemblyInput::Function {
            function: FunctionInput {
                chunks: project::tool_chunks(&function_chunks)?,
            },
        };
        let AssemblyOutput::Function(call) = assemble(&input)? else {
            return Err(AssemblyError::WrongFunctionOutput);
        };
        set_content(draft, None);
        draft.extras.function_call = Some(call);
    }
    Ok(())
}

fn apply_content(draft: &mut Draft, chunks: &[Chunk], now: i64) -> Result<(), AssemblyError> {
    let content = select(chunks, |delta| delta.content.is_some());
    if !content.is_empty() {
        let joined =
            get_combined_content(&project::content_chunks(&content)?, ContentField::Content)?;
        set_content(draft, Some(joined));
    }
    let thinking = select(chunks, |delta| delta.thinking_blocks.is_some());
    if !thinking.is_empty() {
        draft.extras.thinking_blocks =
            get_combined_thinking_content(&project::content_chunks(&thinking)?)?;
    }
    let reasoning = select(chunks, |delta| delta.reasoning_content.is_some());
    if !reasoning.is_empty() {
        let joined = get_combined_content(
            &project::content_chunks(&reasoning)?,
            ContentField::Reasoning,
        )?;
        draft.extras.reasoning_content = Some(joined);
    }
    let audio = select(chunks, |delta| delta.audio.is_some());
    if !audio.is_empty() {
        let joined = get_combined_audio_content(&project::content_chunks(&audio)?, now)?;
        draft.extras.audio = Some(joined);
    }
    Ok(())
}

// LiteLLM assigns through Choices, which owns the one message of the base choice.
fn set_content(draft: &mut Draft, content: Option<String>) {
    if let Some(choice) = draft.base.choices.first_mut() {
        choice.message.content = content;
    }
}

fn first_delta(chunk: &Chunk) -> Option<&Delta> {
    chunk
        .choices
        .first()
        .into_iter()
        .find_map(|choice| choice.delta.as_ref())
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

// The chunks whose first choice carries the field, in stream order.
fn select<K>(chunks: &[Chunk], keep: K) -> Vec<&Chunk>
where
    K: Fn(&Delta) -> bool,
{
    chunks
        .iter()
        .filter(|chunk| first_delta(chunk).is_some_and(&keep))
        .collect()
}

// Every selected first-choice list is concatenated in stream order, and an empty join still assigns.
fn joined_lists<F>(chunks: &[Chunk], field: F) -> Option<Vec<JsonValue>>
where
    F: Fn(&Delta) -> Option<&[JsonValue]>,
{
    let lists: Vec<&[JsonValue]> = chunks
        .iter()
        .filter_map(|chunk| first_delta(chunk).and_then(&field))
        .collect();
    (!lists.is_empty()).then(|| lists.concat())
}

fn joined_provider_fields(chunks: &[Chunk]) -> Option<JsonObject> {
    let selected: Vec<&Choice> = select(chunks, |delta| delta.provider_specific_fields.is_some())
        .into_iter()
        .filter_map(|chunk| chunk.choices.first())
        .collect();
    combined_provider_fields(&selected)
}
