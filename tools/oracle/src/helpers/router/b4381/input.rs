use betterlite_router::semantic_embedding_inputs::{MetadataMap, ModelListEntry};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) embedding_model: String,
    pub(crate) prompt: String,
    pub(crate) metadata: Option<MetadataMap>,
    pub(crate) embedding_max_input_tokens: Option<i64>,
    pub(crate) embedding_timeout: Option<f64>,
    pub(crate) router_present: bool,
    pub(crate) model_list: Option<Vec<ModelRow>>,
    pub(crate) method: Method,
    pub(crate) outcome: Outcome,
    pub(crate) vectors: Vec<Vec<f64>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelRow {
    pub(crate) model_name: String,
    pub(crate) max_input_tokens: Option<i64>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Method {
    Sync,
    Async,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Success,
    Failure,
}

impl Input {
    pub(crate) fn model_entries(&self) -> Option<Vec<ModelListEntry>> {
        self.model_list.as_ref().map(|rows| {
            rows.iter()
                .map(|row| ModelListEntry {
                    model_name: Some(row.model_name.clone()),
                })
                .collect()
        })
    }
}
