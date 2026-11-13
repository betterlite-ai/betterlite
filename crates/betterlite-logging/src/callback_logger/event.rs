use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use indexmap::IndexMap;

use crate::fields::payload::StandardLoggingPayload;

// the fingerprint ties the body to the guardrail history that produced it, so a stale snapshot is dropped
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuardrailRequestSnapshot {
    pub body: IndexMap<String, Json>,
    pub fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct LoggerEvent {
    pub payload: Arc<StandardLoggingPayload>,
    pub dynamic_params: Arc<StandardCallbackDynamicParams>,
    pub stream: bool,
    pub guardrail_snapshot: Option<Arc<GuardrailRequestSnapshot>>,
}
