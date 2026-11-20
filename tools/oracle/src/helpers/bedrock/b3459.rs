use std::collections::BTreeMap;

use betterlite_config::rag::document::ingest::{
    RAGIngestEmbeddingOptions, S3VectorsVectorStoreOptions,
};
use serde::Deserialize;
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod recorder;
mod steps;

use self::steps::{dimension, embed, flow, settings, store};

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub(crate) enum Input {
    Settings { cases: Vec<SettingsCase> },
    Dimension { cases: Vec<DimensionCase> },
    Flow { cases: Vec<FlowCase> },
    Embed(EmbedStep),
    Store(StoreStep),
}

#[derive(Debug, Deserialize)]
pub(crate) struct SettingsCase {
    pub(crate) vector: S3VectorsVectorStoreOptions,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) embedding: Option<RAGIngestEmbeddingOptions>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DimensionCase {
    pub(crate) vector: S3VectorsVectorStoreOptions,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) embedding: Option<RAGIngestEmbeddingOptions>,
    #[serde(default)]
    pub(crate) answer: Vec<Vec<f64>>,
    pub(crate) failing: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FlowCase {
    pub(crate) vector: S3VectorsVectorStoreOptions,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) embedding: Option<RAGIngestEmbeddingOptions>,
    #[serde(default)]
    pub(crate) answer: Vec<Vec<f64>>,
    #[serde(default)]
    pub(crate) filename: Option<String>,
    pub(crate) chunks: Vec<String>,
    pub(crate) embeddings: Vec<Vec<f64>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EmbedStep {
    pub(crate) vector: S3VectorsVectorStoreOptions,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) embedding: Option<RAGIngestEmbeddingOptions>,
    pub(crate) chunk_sets: Vec<Vec<String>>,
    #[serde(default)]
    pub(crate) answer: Vec<Vec<f64>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StoreStep {
    pub(crate) vector: S3VectorsVectorStoreOptions,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) embedding: Option<RAGIngestEmbeddingOptions>,
    pub(crate) calls: Vec<StoreCall>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StoreCall {
    #[serde(default)]
    pub(crate) filename: Option<String>,
    pub(crate) chunks: Vec<String>,
    #[serde(default)]
    pub(crate) embeddings: Option<Vec<Vec<f64>>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        match input {
            Input::Settings { cases } => Ok(settings(cases)),
            Input::Dimension { cases } => dimension(&runtime, cases),
            Input::Flow { cases } => flow(&runtime, cases),
            Input::Embed(step) => embed(&runtime, step),
            Input::Store(step) => store(&runtime, step),
        }
    }
}
