use betterlite_config::sections::grayswan::Configured;
use reqwest::Client;

use crate::guardrail_base::GuardrailBase;

pub mod config_model;
pub mod construction;
pub mod error;
mod format;
pub mod grayswan_monitoring;
mod guardrail;
pub mod hooks;
pub mod wire;

// LiteLLM's GraySwanGuardrail instance: its resolved settings, its guardrail base and the shared HTTP client
#[derive(Debug, Clone)]
pub struct GraySwanGuardrail {
    pub settings: Configured,
    pub base: GuardrailBase,
    pub client: Client,
}

#[path = "grayswan/tests.rs"]
mod tests;
