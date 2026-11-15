use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials::{
    Raw as OpenAiCredentialsRaw, Settings as OpenAiCredentialsSettings,
};
use betterlite_openai::containers::OpenAiContainerConfig;
use betterlite_openai::containers::costs::ContainerCosts;
use betterlite_openai::credentials::LiteLlmGlobals;
use rust_decimal::Decimal;
use secrecy::SecretString;

use super::input::Globals;
use crate::helpers::{HelperError, OrHelperError as _};

// B068 owns the arithmetic; this fixture answers the one lookup the container create response makes
#[derive(Debug)]
pub(super) struct FixtureCost;

impl ContainerCosts for FixtureCost {
    fn code_interpreter_cost(&self, sessions: u32, provider: &str) -> Decimal {
        if sessions == 1 && provider == "openai" {
            Decimal::new(3, 2)
        } else {
            Decimal::ZERO
        }
    }
}

pub(super) fn call_config(
    call_globals: &Globals,
    environment: &BTreeMap<String, String>,
) -> Result<OpenAiContainerConfig, Box<HelperError>> {
    let settings: OpenAiCredentialsSettings = OpenAiCredentialsRaw::default()
        .convert(environment)
        .or_config_error()?;
    let env_api_key = environment
        .get("OPENAI_API_KEY")
        .map(|key| SecretString::from(key.as_str()));
    Ok(OpenAiContainerConfig {
        settings,
        globals: LiteLlmGlobals {
            api_base: call_globals.api_base.clone(),
            api_key: call_globals.api_key.as_deref().map(SecretString::from),
            openai_key: call_globals.openai_key.as_deref().map(SecretString::from),
            organization: None,
        },
        env_api_key,
        costs: Arc::new(FixtureCost),
    })
}
