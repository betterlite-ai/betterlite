pub mod ranking;
pub mod reply;
pub mod request;
pub mod retrieval;

mod tests;

use thiserror::Error;

use crate::api::rerank_contracts::BaseLlmException;

pub const DEFAULT_NIM_RERANK_API_BASE: &str = "https://ai.api.nvidia.com";
const NVIDIA_NIM_PREFIX: &str = "nvidia_nim/";
const RANKING_PREFIX: &str = "ranking/";

const SUPPORTED_COHERE_PARAMS: [&str; 3] = ["query", "documents", "top_n"];
const TEXT_PASSAGE_FIELDS: [&str; 1] = ["text"];
const RANKING_PASSAGE_FIELDS: [&str; 2] = ["text", "image"];

#[derive(Debug, Error)]
pub enum NvidiaNimRerankError {
    #[error(
        "Nvidia NIM API key is required. Please set 'NVIDIA_NIM_API_KEY' in your environment"
    )]
    MissingApiKey,
    #[error("query is required for Nvidia NIM rerank")]
    MissingQuery,
    #[error("documents is required for Nvidia NIM rerank")]
    MissingDocuments,
    #[error("the documents parameter is not a list")]
    DocumentsNotAList,
    #[error("the document at position {position} is not a string or an object")]
    UnsupportedDocument { position: usize },
    #[error("top_n must be a positive integer, got: {value}")]
    TopNNotPositive { value: String },
    #[error("the rerank reply body is not JSON")]
    ReplyNotJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the rerank reply does not match the Nvidia NIM format: {source}")]
    ReplyShape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the rerank reply index {index} is below minus the {passages} passages")]
    RankingIndex { index: i64, passages: usize },
    #[error("the rerank reply document text is not a string")]
    DocumentTextNotString,
    #[error("a value does not serialize: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Upstream(#[from] BaseLlmException),
}

impl NvidiaNimRerankError {
    #[must_use]
    pub fn from_json(source: serde_json::Error) -> Self {
        if source.is_data() {
            Self::ReplyShape { source }
        } else {
            Self::ReplyNotJson { source }
        }
    }
}

pub enum NvidiaNimRerankSelection {
    Retrieval(retrieval::NvidiaNimRerankConfig),
    Ranking(ranking::NvidiaNimRankingConfig),
}

#[must_use]
pub fn select_nvidia_nim_rerank_config(model: &str) -> NvidiaNimRerankSelection {
    if model.starts_with(RANKING_PREFIX) {
        NvidiaNimRerankSelection::Ranking(ranking::NvidiaNimRankingConfig::default())
    } else {
        NvidiaNimRerankSelection::Retrieval(retrieval::NvidiaNimRerankConfig::default())
    }
}
