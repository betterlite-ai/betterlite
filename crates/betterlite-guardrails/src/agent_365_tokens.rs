use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::SecretString;

use crate::agent_365_tokens::cache::{OboTokenCache, clock_seconds};
use crate::agent_365_tokens::error::Agent365Error;
use crate::agent_365_tokens::poster::Agent365Poster;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};

pub mod cache;
pub mod error;
pub mod exchange;
pub mod payload;
pub mod poster;

mod tests;

pub const EVALUATE_URL: &str =
    "https://agent365.svc.cloud.microsoft/agents/tool-evaluation/evaluate";
pub const DEFENDER_STATUS_EVALUATED: &str = "Evaluated";
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 10.0;
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 1] = [GuardrailEventHooks::PreMcpCall];

// LiteLLM's CustomGuardrail subclass sets records_own_guardrail_information and nothing else on its switches
const AGENT_365_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: true,
    uses_apply_guardrail_interface: false,
    deny_text_on_error: false,
};

#[derive(Debug, Clone)]
pub struct Agent365Settings {
    pub guardrail_name: String,
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub request_timeout: Option<f64>,
    pub unreachable_fallback: UnreachableFallback,
    pub event_hook: Option<LitellmParamsMode>,
    pub default_on: bool,
}

pub struct Agent365Guardrail {
    pub base: GuardrailBase,
    pub settings: Agent365Settings,
    pub poster: Arc<dyn Agent365Poster>,
    pub request_timeout: Duration,
    pub cache: Mutex<OboTokenCache>,
    pub clock: Arc<dyn Fn() -> f64 + Send + Sync>,
}

impl Agent365Guardrail {
    pub fn new(
        settings: Agent365Settings,
        poster: Arc<dyn Agent365Poster>,
        strict_modes: bool,
    ) -> Result<Self, Agent365Error> {
        // LiteLLM's constructor default is 10.0 seconds when the config carries no timeout
        let seconds = settings.request_timeout.unwrap_or(DEFAULT_TIMEOUT_SECONDS);
        let request_timeout = match Duration::try_from_secs_f64(seconds) {
            Ok(timeout) => timeout,
            Err(_) => return Err(Agent365Error::Timeout { seconds }),
        };
        let base = GuardrailBase::new(guardrail_settings(&settings), strict_modes)?;
        Ok(Self {
            base,
            settings,
            poster,
            request_timeout,
            cache: Mutex::new(OboTokenCache::new()),
            clock: Arc::new(|| clock_seconds(SystemTime::now())),
        })
    }
}

fn guardrail_settings(settings: &Agent365Settings) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: Some(settings.guardrail_name.clone()),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: settings.event_hook.clone(),
        default_on: settings.default_on,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        // LiteLLM constructs this guardrail with run_in_parallel=True
        run_in_parallel: true,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: None,
        switches: AGENT_365_SWITCHES,
    }
}
