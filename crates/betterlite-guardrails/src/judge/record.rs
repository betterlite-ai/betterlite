use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use crate::content_filter::risk_scoring::GuardrailStatus;

use super::hooks::EventHookConfig;

pub const JUDGE_PROVIDER: &str = "llm_as_a_judge";

// LiteLLM's StandardLoggingGuardrailInformation for one judge run; the judge never sets a duration or a masked entity count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeGuardrailRecord {
    pub guardrail_name: String,
    pub guardrail_provider: String,
    pub guardrail_mode: EventHookConfig,
    pub guardrail_response: Json,
    pub guardrail_status: GuardrailStatus,
    pub start_time: f64,
    pub end_time: f64,
}
