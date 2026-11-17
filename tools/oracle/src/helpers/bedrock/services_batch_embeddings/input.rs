use serde::Deserialize;

use betterlite_bedrock::model_routes::ParamMap;
use betterlite_prompts::arguments::json::{Json, Object};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ClassifyRecords(ClassifyRecords),
    RegistrySchema(RegistrySchema),
    NameBoundary(NameBoundary),
    CoerceInput(CoerceInput),
    MapEmbedding(MapEmbedding),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassifyRecords {
    pub(crate) records: Vec<Object>,
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
    pub(crate) input: Option<Json>,
    pub(crate) model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MapEmbedding {
    pub(crate) model_cost: ParamMap,
    pub(crate) model: String,
    pub(crate) body: Object,
}
