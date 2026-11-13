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
    match task_type {
        None | Some(Value::Null) => source
            .tag(model, None)
            .map_err(HuggingfaceEmbeddingError::Huggingface),
        Some(Value::String(task)) if EMBEDDING_TASKS.contains(&task.as_str()) => {
            Ok(Some(task.clone()))
        }
        Some(Value::String(task)) => Err(HuggingfaceEmbeddingError::InvalidTaskType {
            task_type: task.clone(),
        }),
        Some(other @ (Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_))) => {
            Err(HuggingfaceEmbeddingError::InvalidTaskType {
                task_type: other.to_string(),
            })
        }
    }
}
