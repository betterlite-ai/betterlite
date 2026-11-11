use super::error::SigningError;
use crate::aws_endpoints::error::AwsEndpointError;
use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_settings::sources::Credentials;
use crate::aws_settings::{AuthParams, pop_aws_auth_params};
use crate::credentials::error::CredentialError;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use secrecy::SecretString;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use std::future::Future;
#[derive(Debug)]
pub struct RequestTarget {
    pub region: String,
    pub endpoint: Option<String>,
    pub credentials: Option<Credentials>,
}
#[derive(Debug)]
pub struct TargetRequest<'source> {
    pub optional_params: &'source mut BTreeMap<String, Box<RawValue>>,
    pub model: Option<&'source str>,
    pub region_inputs: RegionInputs<'source>,
    pub bearer_token: Option<&'source SecretString>,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct RegionInputs<'source> {
    pub environment_region_name: Option<&'source str>,
    pub environment_region: Option<&'source str>,
    pub session_region: Option<&'source str>,
}
pub async fn request_target<F, Fut>(
    request: TargetRequest<'_>,
    resolve_credentials: F,
) -> Result<RequestTarget, SigningError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let region = resolve_region(
        request.optional_params,
        request.model,
        request.region_inputs,
    )?;
    request.optional_params.remove("aws_region_name");
    let auth = pop_aws_auth_params(request.optional_params)?;
    let endpoint = request
        .optional_params
        .remove("aws_bedrock_runtime_endpoint")
        .as_deref()
        .map_or(Ok(None), |raw| {
            parse_string(raw, "aws_bedrock_runtime_endpoint")
        })?;
    let credentials = if request.bearer_token.is_some() {
        None
    } else {
        let settings = auth_settings(auth, &region);
        let resolved = resolve_credentials(&settings, &region).await;
        Some(resolved.map_err(SigningError::Credential)?)
    };
    Ok(RequestTarget {
        region,
        endpoint,
        credentials,
    })
}
pub(crate) fn resolve_region(
    parameters: &BTreeMap<String, Box<RawValue>>,
    model: Option<&str>,
    inputs: RegionInputs<'_>,
) -> Result<String, SigningError> {
    let explicit = read_string(parameters, "aws_region_name")?;
    llm_region_name(LlmRegion {
        explicit: explicit.as_deref(),
        model,
        model_id: None,
        environment_region_name: inputs.environment_region_name,
        environment_region: inputs.environment_region,
        session_region: inputs.session_region,
    })
    .map_err(SigningError::Endpoint)
}
pub(crate) fn auth_settings(auth: AuthParams, region: &str) -> AwsSettings {
    AwsSettings {
        access_key_id: auth.aws_access_key_id,
        secret_access_key: auth.aws_secret_access_key,
        session_token: auth.aws_session_token,
        region_name: Some(region.to_owned()),
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
    }
}
fn parse_string(raw: &RawValue, field: &'static str) -> Result<Option<String>, SigningError> {
    serde_json::from_str::<Option<String>>(raw.get())
        .map_err(|source| SigningError::Endpoint(AwsEndpointError::Field { field, source }))
}
fn read_string(
    parameters: &BTreeMap<String, Box<RawValue>>,
    field: &'static str,
) -> Result<Option<String>, SigningError> {
    parameters
        .get(field)
        .map_or(Ok(None), |raw| parse_string(raw, field))
}
