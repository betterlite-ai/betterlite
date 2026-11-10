use serde_json::Value;

use crate::huggingface_chat::error::HuggingfaceError;
use crate::huggingface_embedding::error::HuggingfaceEmbeddingError;

pub const EMBEDDING_TASKS: [&str; 5] = [
    "sentence-similarity",
    "feature-extraction",
    "rerank",
    "embed",
    "similarity",
];

pub trait PipelineTagSource {
    fn tag(&self, model: &str, task_type: Option<&str>)
    -> Result<Option<String>, HuggingfaceError>;
}

pub fn resolve_task(
    model: &str,
    task_type: Option<&Value>,
    source: &dyn PipelineTagSource,
) -> Result<Option<String>, HuggingfaceEmbeddingError> {
    let validated = match task_type {
        None | Some(Value::Null) => None,
        Some(Value::String(task)) if EMBEDDING_TASKS.contains(&task.as_str()) => Some(task.clone()),
        Some(other) => {
            let invalid = match other {
                Value::String(task) => task.clone(),
                Value::Null
                | Value::Bool(_)
                | Value::Number(_)
                | Value::Array(_)
                | Value::Object(_) => other.to_string(),
            };
            return Err(HuggingfaceEmbeddingError::InvalidTaskType { task_type: invalid });
        }
    };
    source
        .tag(model, validated.as_deref())
        .map_err(HuggingfaceEmbeddingError::Huggingface)
}
