pub mod analyze;
pub mod config;
pub mod hooks;
pub mod stream;
pub mod verdict;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_logging::verbose_hooks::VerboseHooks;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_redact::structured::Payload;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::settings::{GuardrailSettings, GuardrailSwitches};
use crate::repello_content::SUPPORTED_EVENT_HOOKS;
use crate::repello_content::config::RepelloAIGuardrailConfigModel;

use config::{RepelloAIParams, RepelloAISettings, RepelloLifecycleError, event_hook_from_mode};
use stream::{SilentConsole, SilentLogger};

pub type Data = BTreeMap<String, GuardrailValue>;

pub const DEFAULT_API_BASE: &str = "https://argusapi.repello.ai/sdk/v1";
pub const API_BASE_ENV: &str = "REPELLOAI_API_BASE";
pub const API_KEY_ENV: &str = "ARGUS_API_KEY";
pub const FALLBACK_API_KEY_ENV: &str = "REPELLOAI_API_KEY";
pub const API_KEY_HEADER: &str = "X-API-Key";
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub const PROMPT_STAGE: &str = "prompt";
pub const RESPONSE_STAGE: &str = "response";

pub const BLOCKED_VERDICT: &str = "blocked";
pub const FLAGGED_VERDICT: &str = "flagged";
pub const PASSED_VERDICT: &str = "passed";
pub const CONFIG_ERROR_STATUS_CODES: [u16; 5] = [400, 401, 403, 404, 422];
pub const BLOCKED_STATUS: u16 = 400;
pub const ERROR_STATUS: u16 = 500;

pub const MISCONFIGURED_MESSAGE: &str = "RepelloAI Argus guardrail is misconfigured";
pub const UNREACHABLE_MESSAGE: &str = "RepelloAI Argus guardrail unreachable";
pub const INVALID_JSON_MESSAGE: &str = "RepelloAI Argus guardrail returned invalid JSON";
pub const FAILED_MESSAGE: &str = "RepelloAI Argus guardrail failed";
pub const BLOCKED_MESSAGE: &str = "Blocked by RepelloAI Argus guardrail";
pub const BLOCKED_DETAIL: &str = "Blocked by RepelloAI Argus guardrail.";
pub const POLICIES_PREFIX: &str = "Blocked by RepelloAI Argus guardrail. Policies violated: ";
pub const UNKNOWN_POLICY: &str = "unknown_policy";
pub const CONFIG_MODEL_NAME: &str = "RepelloAIGuardrailConfigModel";

// The detail keys LiteLLM leaves implicit; betterlite names the failure so a client can distinguish them.
pub const MISCONFIGURED_ERROR_TYPE: &str = "repelloai_guardrail_is_misconfigured";
pub const UNREACHABLE_ERROR_TYPE: &str = "repelloai_guardrail_unreachable";
pub const INVALID_JSON_ERROR_TYPE: &str = "repelloai_guardrail_returned_invalid_json";

pub struct RepelloAILifecycle {
    pub base: GuardrailBase,
    pub settings: RepelloAISettings,
    pub client: reqwest::Client,
    pub clock: fn() -> SystemTime,
    pub hooks: VerboseHooks<SilentLogger, SilentConsole>,
}

impl GuardrailContract for RepelloAILifecycle {
    fn get_config_model(&self) -> Option<GuardrailValue> {
        Some(GuardrailValue::Object(BTreeMap::from([
            (
                "model".to_owned(),
                GuardrailValue::Text(CONFIG_MODEL_NAME.to_owned()),
            ),
            (
                "ui_friendly_name".to_owned(),
                GuardrailValue::Text(RepelloAIGuardrailConfigModel::ui_friendly_name().to_owned()),
            ),
        ])))
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}

pub trait CallbackRegistry {
    fn add_litellm_callback(&mut self, guardrail: Arc<RepelloAILifecycle>);
}

// LiteLLM's initialize_guardrail: the callback joins the logging callback manager before it is returned.
pub fn initialize_guardrail<E, R>(
    params: &RepelloAIParams,
    env: &E,
    client_builder: reqwest::ClientBuilder,
    registry: &mut R,
) -> Result<Arc<RepelloAILifecycle>, RepelloLifecycleError>
where
    E: EnvironmentLookup,
    R: CallbackRegistry,
{
    let event_hook = event_hook_from_mode(&params.mode)?;
    let settings = RepelloAISettings::resolve(params, env)?;
    let client = client_builder
        .build()
        .map_err(|source| RepelloLifecycleError::Client { source })?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|source| RepelloLifecycleError::Redactor { source })?;
    let hooks = VerboseHooks {
        logger: SilentLogger,
        console: SilentConsole,
        set_verbose: Payload::Bool(false),
        redactor: Arc::new(redactor),
    };
    let base = GuardrailBase::new(guardrail_settings(params, event_hook), params.strict_modes)?;
    let guardrail = Arc::new(RepelloAILifecycle {
        base,
        settings,
        client,
        clock: SystemTime::now,
        hooks,
    });
    registry.add_litellm_callback(Arc::clone(&guardrail));
    Ok(guardrail)
}

// The Repello hooks are undecorated, so _call_analyze's finally is the only record it writes.
const REPELLO_SWITCHES: GuardrailSwitches = GuardrailSwitches {
    use_native_during_call_hook: false,
    use_native_lifecycle_hooks: false,
    records_own_information: true,
    uses_apply_guardrail_interface: false,
    deny_text_on_error: false,
};

#[must_use]
pub fn guardrail_settings(
    params: &RepelloAIParams,
    event_hook: LitellmParamsMode,
) -> GuardrailSettings {
    GuardrailSettings {
        guardrail_name: params.guardrail_name.clone(),
        supported_event_hooks: Some(SUPPORTED_EVENT_HOOKS.to_vec()),
        event_hook: Some(event_hook),
        default_on: params.default_on,
        mask_request_content: false,
        mask_response_content: false,
        violation_message_template: None,
        end_session_after_n_fails: None,
        on_violation: None,
        realtime_violation_message: None,
        on_sensitive_data: None,
        sensitive_data_route_to_model: None,
        sticky_session_routing: true,
        run_in_parallel: false,
        scan_raw_request: false,
        only_scan_new_messages: false,
        timeout: params.timeout,
        switches: REPELLO_SWITCHES,
    }
}
