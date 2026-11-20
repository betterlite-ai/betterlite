use betterlite_config::rag::document::ingest::VertexAIVectorStoreOptions;
use betterlite_core::rag::ingestion::config::IngestionOptions;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) options: Value,
    #[serde(default)]
    pub(crate) file: FileInput,
    #[serde(default)]
    pub(crate) responses: Vec<ReplyInput>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileInput {
    #[serde(default)]
    pub(crate) content_base64: Option<String>,
    #[serde(default)]
    pub(crate) filename: Option<String>,
    #[serde(default)]
    pub(crate) content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    pub(crate) status: u16,
    pub(crate) body: String,
}

// LiteLLM reads the same object twice: once as ingest options and once as the vector store config
pub(crate) fn ingestion_options(options: &Value) -> Result<IngestionOptions, String> {
    serde_json::from_str(&options.to_string()).map_err(|error| error.to_string())
}

pub(crate) fn vector_store_options(options: &Value) -> Result<VertexAIVectorStoreOptions, String> {
    let empty = Value::Object(serde_json::Map::default());
    let store = options.get("vector_store").unwrap_or(&empty);
    serde_json::from_value(store.clone()).map_err(|error| error.to_string())
}
