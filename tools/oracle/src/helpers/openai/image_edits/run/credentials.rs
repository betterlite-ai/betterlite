use betterlite_config::sections::Section as _;
use betterlite_config::sections::openai_credentials;
use betterlite_openai::credentials::{
    ExplicitCredentials, LiteLlmGlobals, OpenAiCredentials, get_openai_credentials,
};
use secrecy::SecretString;

use super::super::input::Probe;
use super::invalid;
use crate::helpers::HelperError;

pub(super) fn resolve(probe: &Probe) -> Result<OpenAiCredentials, Box<HelperError>> {
    let settings = openai_credentials::Raw::default()
        .convert(&probe.environment)
        .map_err(|error| invalid(error.to_string()))?;
    let explicit = ExplicitCredentials {
        api_base: probe.params.api_base.clone(),
        api_key: probe.params.api_key.clone().map(SecretString::from),
        organization: probe.params.organization.clone(),
    };
    let globals = LiteLlmGlobals {
        api_base: probe.globals.api_base.clone(),
        api_key: probe.globals.api_key.clone().map(SecretString::from),
        openai_key: probe.globals.openai_key.clone().map(SecretString::from),
        organization: None,
    };
    let environment_key = probe
        .environment
        .get("OPENAI_API_KEY")
        .filter(|text| !text.is_empty())
        .cloned()
        .map(SecretString::from);
    Ok(get_openai_credentials(
        explicit,
        globals,
        &settings,
        environment_key,
    ))
}
