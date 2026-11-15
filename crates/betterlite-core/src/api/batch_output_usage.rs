use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::token_rates::ModelInfo;
use betterlite_models::usage_sum::Usage as SummedUsage;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::api::api_boundaries::ApiLimits;

pub mod aggregate;
pub mod decode;
pub mod error;
pub mod pricing;
mod tests;
pub mod tokens;
pub mod usage;

pub use decode::DecodedBatchLine;
pub use error::{BatchEntryTokenError, BatchOutputUsageError};
pub use pricing::{BatchCostPricer, BatchRowCost, BatchRowPricer};
pub use tokens::{BatchTokenCounter, count_batch_entry_tokens, estimate_batch_entry_tokens};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchOutputProvider {
    Openai,
    Azure,
    VertexAi,
    HostedVllm,
    Anthropic,
    Bedrock,
    Mistral,
}

impl BatchOutputProvider {
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Azure => "azure",
            Self::VertexAi => "vertex_ai",
            Self::HostedVllm => "hosted_vllm",
            Self::Anthropic => "anthropic",
            Self::Bedrock => "bedrock",
            Self::Mistral => "mistral",
        }
    }

    const fn is_anthropic_family(self) -> bool {
        matches!(self, Self::Anthropic | Self::Bedrock)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BatchOutputUsageInput {
    OutputUsage(BatchOutputUsageRequest),
    VertexUsage(BatchVertexUsageRequest),
}

// An internally tagged variant keeps the tag in its content map, so the request structs deny nothing.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchOutputUsageRequest {
    pub provider: BatchOutputProvider,
    pub model_name: Option<String>,
    pub model_info: Option<ModelInfo>,
    pub content: BatchOutputContent,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchVertexUsageRequest {
    pub model_name: Option<String>,
    pub model_info: Option<ModelInfo>,
    pub content: BatchOutputContent,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BatchOutputContent {
    Bytes { bytes: Vec<u8> },
    Lines { lines: Vec<Json> },
}

// LiteLLM's json.loads call at batch_utils.py:595, one stripped JSONL line at a time; a bad line is data, not a fatal error.
pub fn decode_batch_output_usage(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<Vec<DecodedBatchLine>, BatchOutputUsageError> {
    if bytes.len() > limits.max_json_bytes {
        return Err(BatchOutputUsageError::JsonTooLarge {
            observed: bytes.len(),
            limit: limits.max_json_bytes,
        });
    }
    let mut lines = Vec::new();
    for raw in bytes.split(|byte| *byte == b'\n') {
        let line = decode::trim_python_bytes(raw);
        if line.is_empty() {
            continue;
        }
        if line.len() > limits.max_event_bytes {
            return Err(BatchOutputUsageError::LineTooLarge {
                observed: line.len(),
                limit: limits.max_event_bytes,
            });
        }
        if lines.len() >= limits.max_items {
            return Err(BatchOutputUsageError::TooManyLines {
                limit: limits.max_items,
            });
        }
        lines.push(decode::decode_line(line));
    }
    Ok(lines)
}

#[expect(
    clippy::struct_field_names,
    reason = "the counters keep LiteLLM's Usage names the batch report and oracle cases use"
)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchUsageTotals {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchOutputUsageResult {
    pub cost: Decimal,
    pub prompt_cost: Decimal,
    pub completion_cost: Decimal,
    pub usage: BatchUsageTotals,
    pub models: Vec<String>,
    pub successful_requests: usize,
    pub failed_requests: usize,
    pub diagnostics: Vec<BatchOutputDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BatchOutputDiagnostic {
    MalformedLine,
    NonObjectLine {
        #[serde(rename = "type")]
        kind: &'static str,
    },
    UncostableLine {
        custom_id: Option<Json>,
    },
    UnparsedUsage {
        provider: &'static str,
        usage_keys: Vec<String>,
    },
}

// The provider-owned piece of one native Vertex row: its usage, the model that prices it, and its costs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VertexRowStats {
    pub usage: SummedUsage,
    pub total_tokens: i64,
    pub model: Option<String>,
    pub prompt_cost: Decimal,
    pub completion_cost: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConverseUsageOutcome {
    NotConverse,
    Usage(SummedUsage),
    Invalid,
}

// bedrock and vertex depend on this crate, so the provider shapes they own arrive through this seam.
pub trait BatchOutputProviders {
    fn bedrock_embedding_usage(
        &self,
        response_body: &IndexMap<String, Json>,
    ) -> Option<SummedUsage>;
    fn bedrock_converse_usage(&self, usage: &Json) -> ConverseUsageOutcome;
    fn vertex_row_stats(
        &self,
        row: &Json,
        model_name: Option<&str>,
        model_info: Option<&ModelInfo>,
        pricer: &dyn BatchRowPricer,
    ) -> Option<VertexRowStats>;
}

pub struct BatchOutputUsage<'service> {
    pub limits: ApiLimits,
    pub lookup: &'service dyn PriceLookup,
    pub resolution: &'service ResolutionContext<'service>,
    pub providers: &'service dyn BatchOutputProviders,
}

impl BatchOutputUsage<'_> {
    pub fn run(
        &self,
        input: &BatchOutputUsageInput,
    ) -> Result<BatchOutputUsageResult, BatchOutputUsageError> {
        match input {
            BatchOutputUsageInput::OutputUsage(request) => {
                aggregate::aggregate_output(self, request)
            }
            BatchOutputUsageInput::VertexUsage(request) => {
                aggregate::vertex::aggregate_vertex(self, request)
            }
        }
    }
}
