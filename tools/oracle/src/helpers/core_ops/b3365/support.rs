use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::{
    Raw as OpenAiCredentialsRaw, Settings as OpenAiCredentialsSettings,
};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::containers::OpenAiContainerConfig;
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rust_decimal::Decimal;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};

const INIT: &str = r#"{"model":"openai/container","call_type":"create_container","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;

#[derive(Debug)]
pub(super) struct NoCosts;

impl ContainerCosts for NoCosts {
    fn code_interpreter_cost(&self, _sessions: u32, _provider: &str) -> Decimal {
        Decimal::ZERO
    }
}

pub(super) fn container_config() -> Result<OpenAiContainerConfig, Box<HelperError>> {
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let settings: OpenAiCredentialsSettings = OpenAiCredentialsRaw::default()
        .convert(&environment)
        .or_config_error()?;
    Ok(OpenAiContainerConfig {
        settings,
        globals: LiteLlmGlobals {
            api_base: None,
            api_key: None,
            openai_key: None,
            organization: None,
        },
        env_api_key: None,
        costs: Arc::new(NoCosts),
    })
}

pub(super) fn logging_state() -> Result<LoggingState, Box<HelperError>> {
    let init: LoggingInit = serde_json::from_str(INIT).or_value_error()?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?;
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor,
        || String::from("generated"),
        Duration::default,
    )
    .or_config_error()
}
