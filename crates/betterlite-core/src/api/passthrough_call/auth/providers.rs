use std::collections::BTreeMap;

use betterlite_azure::passthrough::config::{
    AzurePassthroughConfig, AzurePassthroughEnvironment, validate_environment as azure_validate,
};
use betterlite_azure::passthrough::error::AzurePassthroughError;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_foundry::credentials::error::CredentialsError;
use betterlite_foundry::passthrough::auth::{
    ValidateEnvironmentRequest, validate_environment as azure_ai_validate,
};
use betterlite_openai::gigachat_passthrough::GigachatPassthroughError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_prompts::arguments::json::Object as PromptsObject;
use secrecy::SecretString;

use super::optional_text;
use crate::api::passthrough_call::config::ConfigError;

// the two Azure auth schemes: a deployment api key, or the Entra credentials in litellm_params
pub(super) enum AzureKind {
    Key,
    Entra,
}

pub(super) fn azure_headers(
    kind: AzureKind,
    api_key: Option<&SecretString>,
    params: &Object,
    base: &str,
) -> Result<Headers, ConfigError> {
    match kind {
        AzureKind::Key => {
            let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
            let credentials = deployment_settings(params)?;
            let environment = AzurePassthroughEnvironment {
                headers: Headers::new(),
                api_key: config.get_api_key(api_key),
                params_key: api_key,
                configured_key: None,
                credentials: &credentials,
                refresh_enabled: false,
                provider: None,
                sources: &NoSources,
            };
            Ok(azure_validate(environment)?)
        }
        AzureKind::Entra => {
            let value = serde_json::to_value(params).map_err(|error| ConfigError::ValueError {
                message: error.to_string(),
            })?;
            let prompts: PromptsObject =
                serde_json::from_value(value).map_err(|error| ConfigError::ValueError {
                    message: error.to_string(),
                })?;
            let environment = BTreeMap::new();
            let request = ValidateEnvironmentRequest {
                headers: Headers::new(),
                api_key,
                params: Some(&prompts),
                api_base: Some(base),
                environment: &environment,
                refresh_enabled: false,
                sources: &NoSources,
            };
            Ok(azure_ai_validate(request)?)
        }
    }
}

// GenericLiteLLMParams.model_validate(litellm_params): the Entra fields come from the deployment params
fn deployment_settings(params: &Object) -> Result<DeploymentSettings, ConfigError> {
    Ok(DeploymentSettings {
        azure_ad_token: optional_text(params, "azure_ad_token")?.map(SecretString::from),
        tenant_id: optional_text(params, "tenant_id")?,
        client_id: optional_text(params, "client_id")?,
        client_secret: optional_text(params, "client_secret")?.map(SecretString::from),
        azure_scope: optional_text(params, "azure_scope")?.unwrap_or_default(),
        azure_username: optional_text(params, "azure_username")?,
        azure_password: optional_text(params, "azure_password")?.map(SecretString::from),
    })
}

impl From<AzurePassthroughError> for ConfigError {
    fn from(error: AzurePassthroughError) -> Self {
        match error {
            AzurePassthroughError::Provider(provider) => Self::Provider(provider),
            AzurePassthroughError::Token(token) => status_error(token.status(), token.to_string()),
            error @ (AzurePassthroughError::MissingApiBase
            | AzurePassthroughError::Url(_)
            | AzurePassthroughError::Passthrough(_)) => Self::ValueError {
                message: error.to_string(),
            },
        }
    }
}

impl From<CredentialsError> for ConfigError {
    fn from(error: CredentialsError) -> Self {
        status_error(credentials_status(&error), error.to_string())
    }
}

// the status a failed Entra token fetch carries; the other failures have none
const fn credentials_status(error: &CredentialsError) -> Option<u16> {
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
        Some(status) => ConfigError::Message { status, message },
        None => ConfigError::ValueError { message },
    }
}
