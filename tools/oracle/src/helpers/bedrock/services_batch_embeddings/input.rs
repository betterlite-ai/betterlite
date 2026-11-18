use serde::Deserialize;

use betterlite_bedrock::batch_embeddings::input::EmbeddingInput;
use betterlite_bedrock::model_routes::ParamMap;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ClassifyRecords(ClassifyRecords),
    RegistrySchema(RegistrySchema),
    NameBoundary(NameBoundary),
    CoerceInput(CoerceInput),
    MapEmbedding(MapEmbedding),
}

// each case line is the whole JSONL record, handed to the production record type unchanged
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassifyRecords {
    pub(crate) records: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistrySchema {
    pub(crate) model_cost: ParamMap,
    pub(crate) models: Vec<String>,
}

// the lookup always misses here, so every answer comes from the id alone
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NameBoundary {
    pub(crate) models: Vec<String>,
}

// an absent input and a null one both read as None, the way the record's `.get` reads them
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CoerceInput {
    #[serde(default)]
    pub(crate) input: Option<EmbeddingInput>,
    pub(crate) model: String,
}

// the case body is the OpenAI /v1/embeddings object, handed to the production request type unchanged
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapEmbedding {
    pub(crate) model_cost: ParamMap,
    pub(crate) model: String,
    pub(crate) body: Value,
}
