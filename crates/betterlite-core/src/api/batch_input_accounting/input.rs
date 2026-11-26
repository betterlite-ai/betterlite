use serde::{Deserialize, Serialize};

use crate::api::batch_limit_descriptors::descriptors::BatchLimitDescriptor;
use crate::api::batch_limit_descriptors::input::BatchLimitPrincipal;
use crate::api::request_destination::RequestData;

// The literal set LiteLLM accepts as custom_llm_provider, with its own default
#[derive(Debug, Clone, Copy, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchInputProvider {
    #[default]
    Openai,
    Azure,
    VertexAi,
}

impl BatchInputProvider {
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Azure => "azure",
            Self::VertexAi => "vertex_ai",
        }
    }
}

// One batch file read: the file, its provider, the caller and the descriptors already computed for it
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchInputAccountingInput {
    pub file_id: String,
    #[serde(default)]
    pub custom_llm_provider: BatchInputProvider,
    #[serde(default)]
    pub caller: Option<BatchLimitPrincipal>,
    #[serde(default)]
    pub data: Option<RequestData>,
    #[serde(default)]
    pub descriptors: Vec<BatchLimitDescriptor>,
}
