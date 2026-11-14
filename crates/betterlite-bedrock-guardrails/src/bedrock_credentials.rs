use std::collections::BTreeMap;
use std::future::Future;

use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_endpoints::region::non_llm_region_name;
use betterlite_bedrock::aws_settings::AuthError;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::aws_settings::{AuthParams, pop_aws_auth_params};
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::bedrock_transport::BedrockGuardrail;

#[derive(Debug, thiserror::Error)]
pub enum CredentialLoadError {
    #[error("the Bedrock guardrail parameters cannot be encoded")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error("{field} must be a string")]
    Field { field: &'static str },
    #[error("the AWS credential lookup failed")]
    Credential(#[source] CredentialError),
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RegionEnvironment<'text> {
    pub region_name: Option<&'text str>,
    pub region: Option<&'text str>,
}

pub trait CredentialSource: Send + Sync {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send;
}

pub async fn load_credentials<C>(
    guardrail: &BedrockGuardrail,
    bearer_token: Option<&SecretString>,
    environment: RegionEnvironment<'_>,
    source: &C,
) -> Result<(Option<Credentials>, String), CredentialLoadError>
where
    C: CredentialSource,
{
    let explicit = optional_string(&guardrail.optional_params, "aws_region_name")?;
    let region = non_llm_region_name(
        explicit.as_deref(),
        environment.region_name,
        environment.region,
    )?;
    if bearer_token.is_some() {
        return Ok((None, region));
    }
    let auth = auth_parameters(&guardrail.optional_params)?;
    let settings = AwsSettings {
        access_key_id: auth.aws_access_key_id,
        secret_access_key: auth.aws_secret_access_key,
        session_token: auth.aws_session_token,
        region_name: Some(region.clone()),
        session_name: auth.aws_session_name,
        profile_name: auth.aws_profile_name,
        role_name: auth.aws_role_name,
        web_identity_token: auth.aws_web_identity_token,
        sts_endpoint: auth.aws_sts_endpoint,
        external_id: auth.aws_external_id,
        session_tags: auth.aws_session_tags,
        role_arn: None,
        web_identity_token_file: None,
        bearer_token: None,
    };
    let credentials = source
        .resolve(settings, region.clone())
        .await
        .map_err(CredentialLoadError::Credential)?;
    Ok((Some(credentials), region))
}

fn optional_string(
    params: &OpaqueParams,
    field: &'static str,
) -> Result<Option<String>, CredentialLoadError> {
    match params.0.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(CredentialLoadError::Field { field }),
    }
}

fn auth_parameters(params: &OpaqueParams) -> Result<AuthParams, CredentialLoadError> {
    let mut raw = BTreeMap::new();
    for (key, value) in &params.0 {
        let encoded = serde_json::to_string(value).map_err(CredentialLoadError::Encode)?;
        let raw_value = RawValue::from_string(encoded).map_err(CredentialLoadError::Encode)?;
        raw.insert(key.clone(), raw_value);
    }
    pop_aws_auth_params(&mut raw).map_err(CredentialLoadError::Auth)
}
