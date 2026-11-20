pub(crate) mod create;
pub(crate) mod list;

use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::container_contracts::config::BaseContainerConfig;
use betterlite_redact::secret::Redactor;
use serde::de::DeserializeOwned;

use super::error::{ContainerOpError, ContainerOpFailure, failure};
use super::mock::mock_response;
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;
use crate::containers::container_request_parameters::{
    ContainerProviderConfig, get_provider_container_config,
};

// step 1 of both operations: LiteLLM's mock short circuit, before any provider, logging or encode work
pub(crate) fn mock_answer<T>(
    mock: Option<&Json>,
    expected: &'static str,
    provider: &str,
    redactor: &Redactor,
) -> Result<Option<T>, ContainerOpFailure>
where
    T: DeserializeOwned,
{
    mock_response(mock, expected)
        .map_err(|error| failure(ContainerOpError::Mock(error), provider, redactor))
}

// which operation's no-config wording an unresolvable provider uses
#[derive(Clone, Copy)]
pub(crate) enum ContainerOperation {
    Create,
    List,
}

impl ContainerOperation {
    fn missing(self, provider: String) -> ContainerOpError {
        match self {
            Self::Create => ContainerOpError::Unsupported { provider },
            Self::List => ContainerOpError::ConfigMissing { provider },
        }
    }
}

// step 3 and 4: LlmProviders(custom_llm_provider) parses, then B3361's lookup answers the provider config
pub(crate) fn provider_config<'deps, Configs>(
    configs: &'deps Configs,
    provider: &str,
    operation: ContainerOperation,
) -> Result<&'deps dyn BaseContainerConfig, ContainerOpError>
where
    Configs: ContainerConfigLookup,
{
    if provider.parse::<ProviderId>().is_err() {
        return Err(ContainerOpError::UnknownProvider {
            provider: provider.to_owned(),
        });
    }
    let kind = get_provider_container_config(provider)
        .ok_or_else(|| operation.missing(provider.to_owned()))?;
    configs
        .container_config(kind_id(kind))
        .ok_or_else(|| operation.missing(provider.to_owned()))
}

const fn kind_id(kind: ContainerProviderConfig) -> ProviderId {
    match kind {
        ContainerProviderConfig::OpenAi => ProviderId::Openai,
        ContainerProviderConfig::Azure => ProviderId::Azure,
    }
}

// `timeout or DEFAULT_REQUEST_TIMEOUT`: a zero or an explicit none takes the request timeout, and the call's own default is 600
pub(crate) fn resolved_timeout(
    timeout: Option<f64>,
    request_timeout_seconds: f64,
) -> Result<Duration, ContainerOpError> {
    let seconds = timeout
        .filter(|given| *given != 0.0_f64)
        .unwrap_or(request_timeout_seconds);
    Duration::try_from_secs_f64(seconds)
        .map_err(|source| ContainerOpError::Timeout { seconds, source })
}

// the Json, JsonValue and Object bridges before the provider work; a failure here is local, not a provider answer
pub(crate) fn params_failure(
    error: serde_json::Error,
    provider: &str,
    redactor: &Redactor,
) -> ContainerOpFailure {
    failure(ContainerOpError::Params(error), provider, redactor)
}
