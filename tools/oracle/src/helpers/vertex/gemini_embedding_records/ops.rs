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
    calls: Vec<Call>,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    BatchRequest,
    EmbedRequest,
    BatchReply,
    EmbedReply,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    kind: Kind,
    model: String,
    input: EmbeddingInput,
    optional: Extra,
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
    let mut values = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        values.push(convert(call));
    }
    json!({"calls": values})
}
fn convert(call: &Call) -> Value {
    let outcome = match call.kind {
        Kind::BatchRequest => transform_batch_request(
            &call.input,
            &call.model,
            &call.optional,
            &call.resolved_files,
        )
        .map(|request| json!(request)),
        Kind::EmbedRequest => {
            transform_embed_content(&call.input, &call.optional, &call.resolved_files)
                .map(|request| json!(request))
        }
        Kind::BatchReply => {
            process_batch_reply(&call.input, &call.model, &call.response, &CharacterCount)
                .map(|reply| json!(reply))
        }
        Kind::EmbedReply => process_embed_content_reply(
            &call.input,
            &call.model,
            &call.response,
            &call.resolved_files,
            &CharacterCount,
        )
        .map(|reply| json!(reply)),
    };
    outcome.unwrap_or_else(|_error| json!({"error": true}))
}
