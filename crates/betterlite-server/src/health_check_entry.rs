mod failure;
mod flow;
pub mod ports;
mod tests;

use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::resolution::ResolutionError;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use serde::Serialize;
use thiserror::Error;

use self::ports::{CatalogModes, ProviderResolver};
use crate::health_probe_modes::backend::{
    CheapestChatModels, HealthProbeBackend, OcrHealthDocuments,
};
use crate::health_probe_modes::error::HealthProbeError;
use crate::health_probe_modes::params::HealthProbeParams;
use crate::health_probe_modes::response::HealthCheckResponse;

pub const DEFAULT_MODE: &str = "chat";
const NO_CACHE_KEY: &str = "no-cache";

// what a health check answers: the three rate limit headers, or the error result LiteLLM builds in its except block
#[derive(Debug)]
pub enum HealthCheckOutcome {
    Healthy(HealthCheckResponse),
    // boxed: the failure record is over 200 bytes larger than the three-header response
    Failed(Box<HealthCheckFailure>),
}

#[derive(Debug)]
pub struct HealthCheckFailure {
    pub error: String,
    // None for the missing-mode result, which LiteLLM returns without the key at all
    pub raw_request: Option<RawRequestRecord>,
    pub exception: HealthEntryError,
}

// RawRequestTypedDict: what the provider call recorded about its outgoing request
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawRequestRecord {
    pub raw_request_api_base: Option<String>,
    pub raw_request_body: Option<Json>,
    pub raw_request_headers: Option<Json>,
    pub error: Option<String>,
}

// ahealth_check's own failures; each Display is the exact text LiteLLM puts in front of the stack text
#[derive(Debug, Error)]
pub enum HealthEntryError {
    #[error("model not set")]
    ModelNotSet,
    #[error("{source}")]
    Resolution {
        #[source]
        source: ResolutionError,
    },
    #[error("Mode {mode} not supported. See modes here: https://docs.litellm.ai/docs/proxy/health")]
    UnsupportedMode { mode: String },
    #[error("{source}")]
    Probe {
        #[source]
        source: HealthProbeError,
    },
}

// ahealth_check's dependencies, injected: the transport, the model list, the catalog, the resolver and the redaction state
pub struct HealthCheckEntry {
    pub backend: Arc<dyn HealthProbeBackend>,
    pub cheapest: Arc<dyn CheapestChatModels>,
    pub documents: Arc<dyn OcrHealthDocuments>,
    pub resolver: Arc<dyn ProviderResolver>,
    pub catalog: Arc<dyn CatalogModes>,
    pub redactor: Arc<Redactor>,
    pub redact_secrets: bool,
    pub clock: fn() -> DateTime<FixedOffset>,
    pub next_id: fn() -> String,
}

struct FlowFailure {
    mode: Option<String>,
    error: HealthEntryError,
}

struct PreparedProbe {
    model: String,
    provider: String,
    mode: String,
    params: HealthProbeParams,
}
