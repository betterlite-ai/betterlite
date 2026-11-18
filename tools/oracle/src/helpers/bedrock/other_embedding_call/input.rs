use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Object;
use serde::Deserialize;

// one variant per named case contract; every variant carries the same call and the name records which seed covers it
#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    EmbeddingFamilySelection(Case),
    OrderedMultiInputResult(Case),
    CohereBatchedInput(Case),
    TitanPerItemInput(Case),
    NovaAsyncInvokeStart(Case),
    SigningAuthIsolation(Case),
    EmbeddingTransportError(Case),
}

impl Input {
    pub(crate) fn case(&self) -> &Case {
        match self {
            Self::EmbeddingFamilySelection(case)
            | Self::OrderedMultiInputResult(case)
            | Self::CohereBatchedInput(case)
            | Self::TitanPerItemInput(case)
            | Self::NovaAsyncInvokeStart(case)
            | Self::SigningAuthIsolation(case)
            | Self::EmbeddingTransportError(case) => case,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Case {
    pub(crate) model: String,
    pub(crate) input: Vec<String>,
    #[serde(default)]
    pub(crate) optional_params: Object,
    #[serde(default)]
    pub(crate) api_base: Option<String>,
    #[serde(default)]
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) timeout: Option<f64>,
    #[serde(default)]
    pub(crate) aembedding: bool,
    #[serde(default)]
    pub(crate) drop_params: bool,
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) now: u64,
    #[serde(default)]
    pub(crate) replies: Vec<Reply>,
    #[serde(default)]
    pub(crate) supplied: Supplied,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) failure: Option<Failure>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failure {
    Timeout,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Supplied {
    #[default]
    None,
    Sync,
    Async,
}

impl Reply {
    pub(crate) fn headers(&self) -> Vec<(String, String)> {
        self.headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
}
