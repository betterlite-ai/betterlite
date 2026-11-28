use std::collections::BTreeMap;
use std::time::SystemTime;

use async_trait::async_trait;
use betterlite_config::loader::ConfigError as SettingsError;
use betterlite_config::sections::Section as _;
use betterlite_config::sections::bedrock_aws_endpoints;
use betterlite_config::sections::bedrock_mantle_inputs;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_core::api::passthrough_call::aws::{AwsFlavor, AwsPassthrough};
use betterlite_core::api::passthrough_call::config::ConfigError;
use betterlite_models::provider_defaults_a::policies::MantleEndpoint;
use betterlite_models::resolution::ResolutionRequest;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughSigning, PassthroughUrlRequest,
};
use serde_json::value::RawValue;
use url::Url;

use super::error::PassthroughFailure;
use super::url::{PassthroughEnvironment, RuntimeUrl, optional_text};
use super::{SigningIdentity, sign_headers, signing_region};
use crate::aws_endpoints::runtime::BedrockEndpointType;
use crate::aws_settings::sources::{Credentials, SourceError};
use crate::aws_settings::{AuthError, bedrock_bearer_token, pop_aws_auth_params};
use crate::credentials::CredentialSource;
use crate::credentials::error::CredentialError;
use crate::mantle_inputs::chat_endpoint::{ChatEndpointParams, chat_endpoint_inputs};
use crate::mantle_inputs::error::MantleInputError;
use crate::mantle_inputs::records::MantleGlobals;
use crate::mantle_inputs::region::split_region_prefix;
use crate::mantle_settings::error::MantleSettingsError;
use crate::mantle_settings::passthrough::{
    PassthroughEndpointRequest, passthrough_bearer_token, passthrough_region,
    passthrough_runtime_endpoint,
};
use crate::model_routes::ParamMap;
use crate::signing::target::auth_settings;

mod tests;

const MANTLE_PREFIX: &str = "bedrock_mantle/";

// ProviderConfigManager's Bedrock and Bedrock Mantle pass-through configs over B591's environment sections
pub struct BedrockSdkCall<Source> {
    environment: PassthroughEnvironment,
    globals: MantleGlobals,
    source: Source,
    now: fn() -> SystemTime,
}

impl<Source> BedrockSdkCall<Source> {
    pub fn from_environment(
        environment: &BTreeMap<String, String>,
        credentials: Source,
        now: fn() -> SystemTime,
    ) -> Result<Self, SettingsError> {
        let endpoints = bedrock_aws_endpoints::Raw::default().convert(environment)?;
        let mantle = bedrock_mantle_inputs::Raw::default().convert(environment)?;
        Ok(Self {
            environment: PassthroughEnvironment {
                region_name: endpoints.aws_region_name.clone(),
                region: endpoints.aws_region.clone(),
                session_region: endpoints.session_region,
                runtime_endpoint: endpoints.aws_bedrock_runtime_endpoint,
            },
            globals: MantleGlobals {
                mantle_api_key: mantle.api_key,
                mantle_api_base: mantle.api_base,
                mantle_region: mantle.region,
                aws_bearer_token: mantle.aws_bearer_token,
                aws_region_name: endpoints.aws_region_name,
                aws_region: endpoints.aws_region,
            },
            source: credentials,
            now,
        })
    }
}

#[async_trait]
impl<Source> AwsPassthrough for BedrockSdkCall<Source>
where
    Source: CredentialSource,
{
    fn mantle_endpoint(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<MantleEndpoint, ConfigError> {
        let model = request
            .model
            .strip_prefix(MANTLE_PREFIX)
            .unwrap_or(request.model);
        let (_, base_model) = split_region_prefix(model);
        let inputs = chat_endpoint_inputs(
            ChatEndpointParams {
                model: Some(model),
                api_base: request.api_base,
                api_key: request.api_key,
                aws_region_name: None,
            },
            &self.globals,
            &ParamMap::new(),
        )?;
        Ok(MantleEndpoint {
            api_base: inputs.api_base,
            api_key: inputs.api_key,
            base_model: base_model.to_owned(),
        })
    }

    fn complete_url(
        &self,
        flavor: AwsFlavor,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), ConfigError> {
        match flavor {
            AwsFlavor::Bedrock => {
                Ok(RuntimeUrl::resolve(&self.environment, request)?.get_complete_url(request)?)
            }
            AwsFlavor::Mantle => {
                let region = passthrough_region(request.litellm_params, &self.globals)?;
                let configured =
                    optional_text(request.litellm_params, "aws_bedrock_runtime_endpoint")?;
                let endpoint = passthrough_runtime_endpoint(&PassthroughEndpointRequest {
                    api_base: request.api_base,
                    configured_endpoint: configured.as_deref(),
                    environment_endpoint: self.environment.runtime_endpoint.as_deref(),
                    region: &region,
                    endpoint_type: BedrockEndpointType::Runtime,
                });
                Ok(RuntimeUrl { endpoint }.get_complete_url(request)?)
            }
        }
    }

    async fn sign_request(
        &self,
        flavor: AwsFlavor,
        headers: Headers,
        context: &PassthroughSigning<'_>,
    ) -> Result<(Headers, Option<Vec<u8>>), ConfigError> {
        let selected = match flavor {
            AwsFlavor::Bedrock => self.globals.aws_bearer_token.clone(),
            AwsFlavor::Mantle => passthrough_bearer_token(context.litellm_params, &self.globals),
        };
        let bearer = bedrock_bearer_token(None, selected.as_ref());
        let region = match &bearer {
            Some(_token) => String::new(),
            None => match flavor {
                AwsFlavor::Bedrock => signing_region(&self.environment, context, false)?,
                AwsFlavor::Mantle => passthrough_region(context.litellm_params, &self.globals)?,
            },
        };
        let credentials = match &bearer {
            Some(_token) => None,
            None => {
                let mut raw = raw_params(context.litellm_params)?;
                let auth = pop_aws_auth_params(&mut raw)?;
                let settings = auth_settings(auth, &region);
                Some(
                    self.source
                        .resolve(settings, region.clone())
                        .await
                        .map_err(|error| ConfigError::ValueError {
                            message: error.to_string(),
                        })?,
                )
            }
        };
        let identity = SigningIdentity {
            credentials: credentials.as_ref(),
            bearer_token: bearer.as_ref(),
            now: (self.now)(),
        };
        Ok(sign_headers(headers, context, &identity, &region)?)
    }
}

// betterlite builds no production AWS credential chain yet, so the Python session passes this until B10175 does
pub struct NoCredentialChain;

impl CredentialSource for NoCredentialChain {
    async fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> Result<Credentials, CredentialError> {
        Err(CredentialError::Source(SourceError::MissingProvider))
    }
}

// the call carries litellm_params as a serde_json object, while the AWS helpers read raw JSON values
fn raw_params(params: &Object) -> Result<BTreeMap<String, Box<RawValue>>, ConfigError> {
    let text = serde_json::to_string(params).map_err(|error| ConfigError::ValueError {
        message: error.to_string(),
    })?;
    serde_json::from_str(&text).map_err(|error| ConfigError::ValueError {
        message: error.to_string(),
    })
}

impl From<PassthroughFailure> for ConfigError {
    fn from(error: PassthroughFailure) -> Self {
        Self::ValueError {
            message: error.to_string(),
        }
    }
}

impl From<MantleInputError> for ConfigError {
    fn from(error: MantleInputError) -> Self {
        Self::ValueError {
            message: error.to_string(),
        }
    }
}

impl From<MantleSettingsError> for ConfigError {
    fn from(error: MantleSettingsError) -> Self {
        Self::ValueError {
            message: error.to_string(),
        }
    }
}

impl From<AuthError> for ConfigError {
    fn from(error: AuthError) -> Self {
        Self::ValueError {
            message: error.to_string(),
        }
    }
}
