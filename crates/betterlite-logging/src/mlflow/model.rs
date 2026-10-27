use serde_json::Value;

pub type Kwargs = serde_json::Map<String, Value>;
pub type Timestamp = chrono::DateTime<chrono::Utc>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpanKind {
    Llm,
    Embedding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Ok,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Span {
    pub request_id: String,
    pub span_id: String,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MlflowEvent {
    pub name: String,
    pub attributes: Kwargs,
}

impl MlflowEvent {
    pub fn streaming_chunk(delta: Value) -> Self {
        let mut attributes = Kwargs::new();
        attributes.insert("delta".to_owned(), delta);
        Self {
            name: "streaming_chunk".to_owned(),
            attributes,
        }
    }
}
