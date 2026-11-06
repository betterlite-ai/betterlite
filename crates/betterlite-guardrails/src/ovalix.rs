use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

pub mod checkpoint;
pub mod config_model;
pub mod environment;
pub mod error;
pub mod exception_status;
pub mod guardrail;
pub mod session_identity;

use crate::generic_guardrail_contracts::{
    GuardrailExtraValue, GuardrailMessage, GuardrailToolParam,
};
use session_identity::Clock;

pub const BLOCKED_ACTION_TYPE: &str = "block";
pub const BLOCKED_FALLBACK_MESSAGE: &str = "This message was blocked by Ovalix";
pub const CHECKPOINT_PATH: &str = "/tracking/custom_application/checkpoint";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallHook {
    PreCall,
    PostCall,
}

pub const SUPPORTED_EVENT_HOOKS: [CallHook; 2] = [CallHook::PreCall, CallHook::PostCall];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    #[default]
    Request,
    Response,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Inputs {
    pub texts: Vec<String>,
    pub images: Option<Vec<String>>,
    pub tools: Option<Vec<GuardrailToolParam>>,
    pub tool_calls: Option<Vec<GuardrailExtraValue>>,
    pub structured_messages: Option<Vec<GuardrailMessage>>,
    pub model: Option<String>,
    pub stream_holdback_chars: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Default)]
pub struct Params {
    pub guardrail_name: Option<String>,
    pub tracker_api_base: Option<String>,
    pub tracker_api_key: Option<SecretString>,
    pub application_id: Option<String>,
    pub pre_checkpoint_id: Option<String>,
    pub post_checkpoint_id: Option<String>,
    // LiteLLM's supported_event_hooks constructor kwarg, which only the explicit constructor caller sets
    pub event_hooks: Vec<CallHook>,
    // litellm_params.mode, stored beside the supported hooks the way CustomGuardrail keeps event_hook
    pub mode: Vec<CallHook>,
    pub default_on: bool,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct OvalixGuardrail<C> {
    pub settings: environment::Settings,
    pub guardrail_name: String,
    pub default_on: bool,
    pub event_hooks: Vec<CallHook>,
    pub mode: Vec<CallHook>,
    pub timeout: Duration,
    pub client: reqwest::Client,
    pub clock: C,
}

pub trait CallbackRegistry<C: Clock> {
    fn add_litellm_callback(&mut self, guardrail: Arc<OvalixGuardrail<C>>);
}

pub fn initialize_guardrail<E, C, R>(
    params: Params,
    env: &E,
    clock: C,
    client: reqwest::Client,
    registry: &mut R,
) -> Result<Arc<OvalixGuardrail<C>>, environment::OvalixConfigError>
where
    E: EnvironmentLookup,
    C: Clock,
    R: CallbackRegistry<C>,
{
    let resolved = environment::resolve(&params, env, &params.event_hooks)?;
    let guardrail = Arc::new(OvalixGuardrail {
        settings: resolved.settings,
        guardrail_name: params.guardrail_name.unwrap_or_default(),
        default_on: params.default_on,
        event_hooks: resolved.event_hooks,
        mode: params.mode,
        timeout: params.timeout.unwrap_or(DEFAULT_TIMEOUT),
        client,
        clock,
    });
    registry.add_litellm_callback(Arc::clone(&guardrail));
    Ok(guardrail)
}

mod tests;
