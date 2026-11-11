use betterlite_models::decisions::{Extra, Json};
use betterlite_tokenizer::count::error::CountError;
use betterlite_vertex::gemini_embedding_records::input::{EmbeddingInput, ResolvedFiles};
use betterlite_vertex::gemini_embedding_records::request::{
    transform_batch_request, transform_embed_content,
};
use betterlite_vertex::gemini_embedding_records::response::{
    process_batch_reply, process_embed_content_reply,
};
use betterlite_vertex::gemini_embedding_records::usage::TextTokenCounter;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    batch_requests: Vec<Request>,
    embed_content_requests: Vec<Request>,
    batch_replies: Vec<BatchReply>,
    embed_content_replies: Vec<EmbedReply>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    model: String,
    input: EmbeddingInput,
    optional: Extra,
    resolved_files: ResolvedFiles,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchReply {
    model: String,
    input: EmbeddingInput,
    response: Json,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbedReply {
    model: String,
    input: EmbeddingInput,
    resolved_files: ResolvedFiles,
    response: Json,
}

struct CharacterCount;

impl TextTokenCounter for CharacterCount {
    fn count_text(&self, _model: &str, text: &str) -> Result<u32, CountError> {
        u32::try_from(text.chars().count()).map_err(|_error| CountError::Overflow)
    }
}

pub(super) fn run(input: &Input) -> Value {
    let counter = CharacterCount;
    json!({
        "batch_requests": batch_requests(&input.batch_requests),
        "embed_content_requests": embed_content_requests(&input.embed_content_requests),
        "batch_replies": batch_replies(&input.batch_replies, &counter),
        "embed_content_replies": embed_content_replies(&input.embed_content_replies, &counter),
    })
}

fn batch_requests(entries: &[Request]) -> Vec<Value> {
    let mut values = Vec::with_capacity(entries.len());
    for entry in entries {
        let value = match transform_batch_request(
            &entry.input,
            &entry.model,
            &entry.optional,
            &entry.resolved_files,
        ) {
            Ok(request) => json!(request),
            Err(_error) => json!({"error": true}),
        };
        values.push(value);
    }
    values
}

fn embed_content_requests(entries: &[Request]) -> Vec<Value> {
    let mut values = Vec::with_capacity(entries.len());
    for entry in entries {
        let value =
            match transform_embed_content(&entry.input, &entry.optional, &entry.resolved_files) {
                Ok(request) => json!(request),
                Err(_error) => json!({"error": true}),
            };
        values.push(value);
    }
    values
}

fn batch_replies(entries: &[BatchReply], counter: &CharacterCount) -> Vec<Value> {
    let mut values = Vec::with_capacity(entries.len());
    for entry in entries {
        let value = match process_batch_reply(&entry.input, &entry.model, &entry.response, counter)
        {
            Ok(reply) => json!(reply),
            Err(_error) => json!({"error": true}),
        };
        values.push(value);
    }
    values
}

fn embed_content_replies(entries: &[EmbedReply], counter: &CharacterCount) -> Vec<Value> {
    let mut values = Vec::with_capacity(entries.len());
    for entry in entries {
        let value = match process_embed_content_reply(
            &entry.input,
            &entry.model,
            &entry.response,
            &entry.resolved_files,
            counter,
        ) {
            Ok(reply) => json!(reply),
            Err(_error) => json!({"error": true}),
        };
        values.push(value);
    }
    values
}
