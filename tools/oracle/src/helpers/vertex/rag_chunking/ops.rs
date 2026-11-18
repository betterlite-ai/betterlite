use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_vertex::vertex_rag_ingest::chunking::{
    ChunkDefaults, RagChunkingStrategy, transform_chunking_strategy,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;

use super::warnings::Warnings;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    // missing and null both stand for LiteLLM's falsy strategy, which takes the constants
    #[serde(default)]
    chunking_strategy: Option<RagChunkingStrategy>,
}

pub(super) fn run(input: &Input) -> Result<Value, Box<HelperError>> {
    let mut cases = Vec::with_capacity(input.cases.len());
    for case in &input.cases {
        let warnings = Warnings::default();
        let subscriber = tracing_subscriber::registry().with(warnings.clone());
        let record = with_default(subscriber, || {
            transform_chunking_strategy(case.chunking_strategy.as_ref(), ChunkDefaults::LITELLM)
        });
        cases.push(json!({
            "config": serde_json::to_value(record).or_value_error()?,
            "warnings": warnings.messages(),
        }));
    }
    Ok(json!({"cases": cases}))
}
