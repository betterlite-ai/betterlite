use std::future::Future;

use betterlite_config::sections::bedrock_settings::AwsSettings;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};
use crate::aws_settings::sources::{Credentials, SourceError};
use crate::credentials::AwsCredentialResolver;
use crate::credentials::error::CredentialError;
use crate::signing::target::RegionInputs;

pub struct Preparation<'call> {
    pub model: &'call str,
    pub api_base: Option<&'call str>,
    pub aws_bedrock_runtime_endpoint: Option<&'call str>,
    pub aws_region_name: Option<&'call str>,
    pub region_inputs: RegionInputs<'call>,
}

#[derive(Debug)]
pub struct PreparedTarget {
    pub region: String,
    pub endpoint_uri: String,
}

pub fn prepare(input: Preparation<'_>) -> Result<PreparedTarget, AwsEndpointError> {
    // An explicit region is retained verbatim, empty included; only an absent one falls back
    let region = match input.aws_region_name {
        Some(explicit) => explicit.to_owned(),
        None => llm_region_name(LlmRegion {
            explicit: None,
            model: Some(input.model),
            model_id: None,
            environment_region_name: input.region_inputs.environment_region_name,
            environment_region: input.region_inputs.environment_region,
            session_region: input.region_inputs.session_region,
        })?,
    };
    let endpoint_uri = runtime_endpoint(RuntimeEndpointInput {
        api_base: input.api_base,
        configured_endpoint: input.aws_bedrock_runtime_endpoint,
        environment_endpoint: None,
        region: &region,
        endpoint_type: BedrockEndpointType::Runtime,
    })
    .endpoint_url;
    Ok(PreparedTarget {
        region,
        endpoint_uri,
    })
}

// The credential chain seams the transport needs; None means no credentials were found at all
pub trait RealtimeCredentials: Send + Sync {
    fn resolve(
        &self,
        settings: &AwsSettings,
        region: &str,
    ) -> impl Future<Output = Result<Option<Credentials>, CredentialError>> + Send;
}

impl RealtimeCredentials for AwsCredentialResolver<'_> {
    async fn resolve(
        &self,
        settings: &AwsSettings,
        region: &str,
    ) -> Result<Option<Credentials>, CredentialError> {
        match self.resolve_credentials(settings, Some(region)).await {
            Ok(credentials) => Ok(Some(credentials)),
            Err(CredentialError::Source(SourceError::MissingProvider)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}
