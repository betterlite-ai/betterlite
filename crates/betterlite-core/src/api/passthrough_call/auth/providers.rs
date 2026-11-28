use std::collections::BTreeMap;

use betterlite_azure::passthrough::config::{
    AzurePassthroughEnvironment, validate_environment as azure_validate,
};
use betterlite_azure::passthrough::error::AzurePassthroughError;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_foundry::credentials::params::EntraParams;
use betterlite_foundry::passthrough::auth::{
    ValidateEnvironmentRequest, validate_environment as azure_ai_validate,
};
use betterlite_openai::gigachat_passthrough::GigachatPassthroughError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_prompts::arguments::json::Object as PromptsObject;
use secrecy::SecretString;

use crate::api::passthrough_call::config::{ConfigError, azure_globals};

// the two Azure auth schemes: a deployment api key, or the Entra credentials in litellm_params
#[derive(Clone, Copy)]
pub(super) enum AzureKind {
    Key,
    Entra,
}

pub(super) fn azure_headers(
    kind: AzureKind,
    api_key: Option<&SecretString>,
    params: &Object,
    base: &str,
    environment: &BTreeMap<String, String>,
) -> Result<Headers, ConfigError> {
    match kind {
        AzureKind::Key => {
            let azure = azure_globals(environment)?;
            let prompts = prompts_object(params)?;
            let credentials =
                EntraParams::from_mapping(Some(&prompts))?.deployment_settings(environment);
            Ok(azure_validate(AzurePassthroughEnvironment {
                headers: Headers::new(),
                api_key: azure.config.get_api_key(api_key),
                params_key: api_key,
                configured_key: azure
                    .environment
                    .openai_api_key
                    .as_ref()
                    .or(azure.environment.api_key.as_ref()),
                credentials: &credentials,
                refresh_enabled: false,
                provider: None,
                sources: &NoSources,
            })?)
        }
        AzureKind::Entra => {
            let prompts = prompts_object(params)?;
            Ok(azure_ai_validate(ValidateEnvironmentRequest {
                headers: Headers::new(),
                api_key,
                params: Some(&prompts),
                api_base: Some(base),
                environment,
                refresh_enabled: false,
                sources: &NoSources,
            })?)
        }
    }
}

// litellm_params as the typed mapping EntraParams and the Azure AI Entra check read
fn prompts_object(params: &Object) -> Result<PromptsObject, ConfigError> {
    let value = serde_json::to_value(params).map_err(|error| ConfigError::ValueError {
        message: error.to_string(),
    })?;
    serde_json::from_value(value).map_err(|error| ConfigError::ValueError {
        message: error.to_string(),
    })
}

impl From<AzurePassthroughError> for ConfigError {
    fn from(error: AzurePassthroughError) -> Self {
        match error {
            AzurePassthroughError::Provider(provider) => Self::Provider(provider),
            AzurePassthroughError::Token(token) => status_error(token.status(), token.to_string()),
            problem @ (AzurePassthroughError::MissingApiBase
            | AzurePassthroughError::Url(_)
            | AzurePassthroughError::Passthrough(_)) => Self::ValueError {
                message: problem.to_string(),
            },
        }
    }
}

impl From<CredentialsError> for ConfigError {
    fn from(error: CredentialsError) -> Self {
        status_error(credential_status(&error), error.to_string())
    }
}

// the deployment's token and source failures carry the upstream status; the pre-flight errors do not
const fn credential_status(error: &CredentialsError) -> Option<u16> {
    match error {
        CredentialsError::Token(token) => token.status(),
        CredentialsError::Source(failure) => failure.status,
        CredentialsError::MissingCredentials { .. }
        | CredentialsError::AgentTokenUnresolved { .. }
        | CredentialsError::InvalidParam { .. }
        | CredentialsError::TokenNotString { .. }
        | CredentialsError::ProviderFailed { .. }
        | CredentialsError::Secret(_) => None,
    }
}

pub(super) fn gigachat_error(error: GigachatPassthroughError) -> ConfigError {
    let message = error.to_string();
    match error {
        GigachatPassthroughError::Provider(provider) => ConfigError::Provider(provider),
        GigachatPassthroughError::Token(token) => ConfigError::Message {
            status: token.status_code,
            message,
        },
        GigachatPassthroughError::Passthrough(source) => ConfigError::Passthrough(source),
    }
}

pub(super) const fn status_error(status: Option<u16>, message: String) -> ConfigError {
    match status {
        Some(code) => ConfigError::Message {
            status: code,
            message,
        },
        None => ConfigError::ValueError { message },
    }
}
