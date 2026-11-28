use async_trait::async_trait;
use betterlite_openai::openai_base_config::Object;
use betterlite_sigv4::credentials::Credentials;

use super::endpoint::DEFAULT_REGION;
use super::error::SpeechError;
use super::values::text_field;

// the ten source arguments LiteLLM's _sign_polly_request hands get_credentials
#[derive(Clone, Copy)]
pub struct CredentialSources<'text> {
    pub aws_access_key_id: Option<&'text str>,
    pub aws_secret_access_key: Option<&'text str>,
    pub aws_session_token: Option<&'text str>,
    pub aws_region_name: Option<&'text str>,
    pub aws_session_name: Option<&'text str>,
    pub aws_profile_name: Option<&'text str>,
    pub aws_role_name: Option<&'text str>,
    pub aws_web_identity_token: Option<&'text str>,
    pub aws_sts_endpoint: Option<&'text str>,
    pub aws_external_id: Option<&'text str>,
}

// B594's identity acquisition sits behind this seam: betterlite-bedrock owns the resolver and depends on this crate
#[async_trait]
pub trait CredentialResolver: Send + Sync {
    async fn resolve(&self, sources: &CredentialSources<'_>) -> Result<Credentials, SpeechError>;
}

// the signing region reaches get_credentials already defaulted, so an absent key reads us-east-1 and a null one stays null
pub fn sources(params: &Object) -> Result<CredentialSources<'_>, SpeechError> {
    let region = if params.contains_key("aws_region_name") {
        text_field(params, "aws_region_name")?
    } else {
        Some(DEFAULT_REGION)
    };
    Ok(CredentialSources {
        aws_access_key_id: text_field(params, "aws_access_key_id")?,
        aws_secret_access_key: text_field(params, "aws_secret_access_key")?,
        aws_session_token: text_field(params, "aws_session_token")?,
        aws_region_name: region,
        aws_session_name: text_field(params, "aws_session_name")?,
        aws_profile_name: text_field(params, "aws_profile_name")?,
        aws_role_name: text_field(params, "aws_role_name")?,
        aws_web_identity_token: text_field(params, "aws_web_identity_token")?,
        aws_sts_endpoint: text_field(params, "aws_sts_endpoint")?,
        aws_external_id: text_field(params, "aws_external_id")?,
    })
}
