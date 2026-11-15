use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_redact::structured::Payload;
use uuid::Uuid;

use crate::stream_chunk_shaping::tokens::mapped_finish_reason;

use super::AssemblyError;
use super::Request;
use super::records::Chunk;
use super::response::{TEXT_OBJECT, TextChoice, TextResponse, TextUsage};

const COUNT_FAILED: &str = "token_counter failed, assuming prompt tokens is 0";
const GENERATED_ID_PREFIX: &str = "chatcmpl-";

// stream_chunk_builder_text_completion: one choice built from the first chunk's metadata and the last choice.
pub(crate) async fn assemble_text<L, C>(
    request: &Request<'_>,
    chunks: &[Chunk],
    hooks: &VerboseHooks<L, C>,
) -> Result<TextResponse, AssemblyError>
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    let Some(first) = chunks.first() else {
        return Err(AssemblyError::MissingChunks);
    };
    let Some(last_choice) = chunks.last().and_then(|chunk| chunk.choices.first()) else {
        return Err(AssemblyError::MissingLastChoice);
    };
    let text = joined_text(chunks);
    let model = first.model.clone().unwrap_or_default();
    let prompt_tokens = count_prompt(&model, request, hooks).await;
    let completion_tokens = request.counter.count_response_text(&model, &text).await?;
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(AssemblyError::Overflow)?;
    Ok(TextResponse {
        id: first
            .id
            .clone()
            .unwrap_or_else(|| format!("{GENERATED_ID_PREFIX}{}", Uuid::new_v4())),
        object: TEXT_OBJECT.to_owned(),
        created: first.created.unwrap_or(request.now),
        model: first.model.clone(),
        system_fingerprint: first.system_fingerprint.clone(),
        choices: vec![TextChoice {
            text,
            index: 0,
            logprobs: last_choice.logprobs.clone(),
            finish_reason: last_choice
                .finish_reason
                .as_deref()
                .filter(|reason| !reason.is_empty())
                .map(mapped_finish_reason)
                .map(str::to_owned),
        }],
        usage: TextUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        },
    })
}

// Every text choice value that is a string is joined, null values included in the skip.
fn joined_text(chunks: &[Chunk]) -> String {
    let mut text = String::new();
    for chunk in chunks {
        for choice in &chunk.choices {
            if let Some(part) = choice.text_value() {
                text.push_str(part);
            }
        }
    }
    text
}

// A prompt count failure only costs the count, and the verbose line is the hook LiteLLM prints there.
async fn count_prompt<L, C>(model: &str, request: &Request<'_>, hooks: &VerboseHooks<L, C>) -> i64
where
    L: LoggerSink + Sync,
    C: ConsoleSink + Sync,
{
    // stream_chunk_builder_text_completion takes only a messages sequence, so the caller's callback never reaches it.
    let counted = request
        .counter
        .count_messages(model, request.messages)
        .await;
    match counted {
        Ok(count) => count,
        Err(_error) => {
            let _printed = hooks.utils_print_verbose(
                Payload::String(COUNT_FAILED.to_owned()),
                "DEBUG",
                &Payload::Bool(false),
            );
            0
        }
    }
}
