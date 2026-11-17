use serde::Serialize;

use betterlite_prompts::arguments::json::{Json, Object};

const EMBEDDINGS_URL: &str = "/v1/embeddings";
const TEXT_COMPLETIONS_URL: &str = "/v1/completions";
const RESPONSES_URL: &str = "/v1/responses";

// Which OpenAI endpoint shape a Bedrock batch JSONL line carries
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchRecordKind {
    Chat,
    TextCompletion,
    Responses,
    Embedding,
}

// A recognized url decides on its own; any other url stays chat instead of a body guess
#[must_use]
pub fn classify_batch_record(openai_jsonl_record: &Object) -> BatchRecordKind {
    match openai_jsonl_record.get("url") {
        Some(Json::Text(url)) if !url.is_empty() => return kind_for_url(url),
        Some(Json::Text(_) | Json::Null) | None => {}
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return BatchRecordKind::Chat;
        }
    }
    let Some(Json::Object(body)) = openai_jsonl_record.get("body") else {
        return BatchRecordKind::Chat;
    };
    // a malformed row carrying several keys keeps the conversation instead of losing it
    if body.contains_key("messages") {
        return BatchRecordKind::Chat;
    }
    if body.contains_key("prompt") {
        return BatchRecordKind::TextCompletion;
    }
    if body.contains_key("input") {
        return BatchRecordKind::Embedding;
    }
    BatchRecordKind::Chat
}

fn kind_for_url(url: &str) -> BatchRecordKind {
    match url {
        EMBEDDINGS_URL => BatchRecordKind::Embedding,
        TEXT_COMPLETIONS_URL => BatchRecordKind::TextCompletion,
        RESPONSES_URL => BatchRecordKind::Responses,
        _ => BatchRecordKind::Chat,
    }
}
