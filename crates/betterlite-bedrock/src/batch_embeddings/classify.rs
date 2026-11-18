use serde::Deserialize;
use serde::Serialize;
use serde::de::IgnoredAny;

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

// One JSONL line: the url and body this classification reads, and nothing else
#[derive(Debug, Deserialize)]
pub struct BatchRecord {
    #[serde(default)]
    pub url: Option<RecordUrl>,
    #[serde(default)]
    pub body: RecordBody,
}

// Any value the line's url can hold; a non-string is read no further and stays chat
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RecordUrl {
    Text(String),
    Other(IgnoredAny),
}

// A body that is not an object has no keys, so it classifies as chat
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RecordBody {
    Object(BodyFields),
    NotObject(IgnoredAny),
}

impl Default for RecordBody {
    fn default() -> Self {
        Self::NotObject(IgnoredAny)
    }
}

// Only presence matters, and a null value is present, so each key is read as a marker
#[derive(Debug, Default, Deserialize)]
pub struct BodyFields {
    #[serde(default)]
    pub messages: Presence,
    #[serde(default)]
    pub prompt: Presence,
    #[serde(default)]
    pub input: Presence,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(from = "IgnoredAny")]
pub enum Presence {
    #[default]
    Missing,
    Present,
}

impl From<IgnoredAny> for Presence {
    fn from(_ignored: IgnoredAny) -> Self {
        Self::Present
    }
}

// A recognized url decides on its own; any other url stays chat instead of a body guess
#[must_use]
pub fn classify_batch_record(record: &BatchRecord) -> BatchRecordKind {
    match record.url.as_ref() {
        Some(RecordUrl::Text(url)) => match url.as_str() {
            EMBEDDINGS_URL => BatchRecordKind::Embedding,
            TEXT_COMPLETIONS_URL => BatchRecordKind::TextCompletion,
            RESPONSES_URL => BatchRecordKind::Responses,
            "" => body_kind(&record.body),
            _ => BatchRecordKind::Chat,
        },
        Some(RecordUrl::Other(_)) => BatchRecordKind::Chat,
        None => body_kind(&record.body),
    }
}

const fn body_kind(body: &RecordBody) -> BatchRecordKind {
    let RecordBody::Object(fields) = body else {
        return BatchRecordKind::Chat;
    };
    // a malformed row carrying several keys keeps the conversation instead of losing it
    if matches!(fields.messages, Presence::Present) {
        return BatchRecordKind::Chat;
    }
    if matches!(fields.prompt, Presence::Present) {
        return BatchRecordKind::TextCompletion;
    }
    if matches!(fields.input, Presence::Present) {
        return BatchRecordKind::Embedding;
    }
    BatchRecordKind::Chat
}
