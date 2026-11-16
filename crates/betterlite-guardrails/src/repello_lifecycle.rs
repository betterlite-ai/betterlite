pub mod analyze;
pub mod config;
pub mod error;
pub mod hooks;
pub mod initialize;
pub mod records;
pub mod stream;
pub mod verdict;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity as _;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_logging::verbose_hooks::VerboseHooks;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract;
use crate::repello_content::SUPPORTED_EVENT_HOOKS;
use crate::repello_content::config::RepelloAIGuardrailConfigModel;

use config::RepelloAISettings;
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
