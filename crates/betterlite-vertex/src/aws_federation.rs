use std::future::Future;
use std::time::SystemTime;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::AwsCredentialResolver;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::signing::error::SigningError;
use betterlite_bedrock::signing::sigv4::{Signable, sign_request};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use secrecy::SecretString;
use serde::Deserialize;

mod tests;

const TARGET_RESOURCE_HEADER: &str = "x-goog-cloud-target-resource";

// urllib.parse.quote leaves these alone, and google-auth quotes the whole subject token with it
const PYTHON_QUOTE_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~')
    .remove(b'/');

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AwsParams {
    #[serde(rename = "aws_access_key_id")]
    pub access_key_id: Option<SecretString>,
    #[serde(rename = "aws_secret_access_key")]
    pub secret_access_key: Option<SecretString>,
    #[serde(rename = "aws_session_token")]
    pub session_token: Option<SecretString>,
    #[serde(rename = "aws_region_name")]
    pub region_name: Option<String>,
    #[serde(rename = "aws_session_name")]
    pub session_name: Option<String>,
    #[serde(rename = "aws_profile_name")]
    pub profile_name: Option<String>,
    #[serde(rename = "aws_role_name")]
    pub role_name: Option<String>,
    #[serde(rename = "aws_web_identity_token")]
    pub web_identity_token: Option<SecretString>,
    #[serde(rename = "aws_sts_endpoint")]
    pub sts_endpoint: Option<String>,
    #[serde(rename = "aws_external_id")]
    pub external_id: Option<String>,
}

impl AwsParams {
    fn settings(&self) -> AwsSettings {
        AwsSettings {
            access_key_id: self.access_key_id.clone(),
            secret_access_key: self.secret_access_key.clone(),
            session_token: self.session_token.clone(),
            region_name: self.region_name.clone(),
            session_name: self.session_name.clone(),
            profile_name: self.profile_name.clone(),
            role_name: self.role_name.clone(),
            web_identity_token: self.web_identity_token.clone(),
            sts_endpoint: self.sts_endpoint.clone(),
            external_id: self.external_id.clone(),
            session_tags: None,
            role_arn: None,
            web_identity_token_file: None,
            bearer_token: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FederationInput {
    pub audience: Option<String>,
    pub subject_token_type: Option<String>,
    pub token_url: Option<String>,
    #[serde(rename = "service_account_impersonation_url")]
    pub impersonation_url: Option<String>,
    pub universe_domain: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(flatten)]
    pub aws: AwsParams,
}

pub trait AwsCredentialsResolver {
    fn credentials(
        &self,
        settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send;
}

impl AwsCredentialsResolver for AwsCredentialResolver<'_> {
    fn credentials(
        &self,
        settings: &AwsSettings,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.get_credentials(settings, None)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FederationError {
    #[error(
        "aws_region_name is required in the WIF credential when explicit AWS authentication is used"
    )]
    AwsRegionRequired,
    #[error("the AWS credentials could not be resolved")]
    Credentials(#[source] CredentialError),
    #[error("the AWS GetCallerIdentity request could not be signed")]
    Signing(#[source] SigningError),
    #[error("the AWS subject token could not be encoded")]
    Encode(#[source] serde_json::Error),
}

pub struct AwsSecuritySupplier<'resolver, R> {
    pub resolver: &'resolver R,
    pub settings: AwsSettings,
    pub region: String,
}

impl<R> AwsSecuritySupplier<'_, R>
where
    R: AwsCredentialsResolver + Sync,
{
    pub async fn security_credentials(&self) -> Result<Credentials, FederationError> {
        self.resolver
            .credentials(&self.settings)
            .await
            .map_err(FederationError::Credentials)
    }
}

pub struct AwsFederation<'resolver, R> {
    pub supplier: AwsSecuritySupplier<'resolver, R>,
    pub audience: Option<String>,
    pub subject_token_type: Option<String>,
    pub token_url: Option<String>,
    pub impersonation_url: Option<String>,
    pub universe_domain: Option<String>,
    pub scopes: Vec<String>,
}

pub fn credentials_from_explicit_aws<R>(
    input: FederationInput,
    resolver: &R,
) -> Result<AwsFederation<'_, R>, FederationError>
where
    R: AwsCredentialsResolver,
{
    let region = input
        .aws
        .region_name
        .as_deref()
        .filter(|region| !region.is_empty())
        .ok_or(FederationError::AwsRegionRequired)?
        .to_owned();
    let settings = input.aws.settings();
    Ok(AwsFederation {
        supplier: AwsSecuritySupplier {
            resolver,
            settings,
            region,
        },
        audience: input.audience,
        subject_token_type: input.subject_token_type,
        token_url: input.token_url,
        impersonation_url: input.impersonation_url,
        universe_domain: input.universe_domain,
        scopes: input.scopes,
    })
}

impl<R> AwsFederation<'_, R>
where
    R: AwsCredentialsResolver + Sync,
{
    pub async fn subject_token(&self, now: SystemTime) -> Result<SecretString, FederationError> {
        let credentials = self.supplier.security_credentials().await?;
        let region = &self.supplier.region;
        let url = format!(
            "https://sts.{region}.amazonaws.com?Action=GetCallerIdentity&Version=2011-06-15"
        );
        let computed = sign_request(&Signable {
            method: "POST",
            url: &url,
            headers: &[],
            body: b"",
            service: "sts",
            region,
            credentials: &credentials,
            now,
        })
        .map_err(FederationError::Signing)?;
        let headers = subject_token_headers(&computed, self.audience.as_deref(), region);
        let request = SubjectTokenRequest {
            headers,
            method: "POST",
            url,
        };
        let encoded = serde_json::to_string(&request).map_err(FederationError::Encode)?;
        Ok(SecretString::from(
            utf8_percent_encode(&encoded, PYTHON_QUOTE_SAFE).to_string(),
        ))
    }
}

fn subject_token_headers(
    computed: &[(String, String)],
    audience: Option<&str>,
    region: &str,
) -> Vec<SubjectTokenHeader> {
    let mut headers = Vec::with_capacity(5);
    if let Some(value) = computed_header(computed, "Authorization") {
        headers.push(SubjectTokenHeader {
            key: "Authorization",
            value: Some(value.to_owned()),
        });
    }
    headers.push(SubjectTokenHeader {
        key: "host",
        value: Some(format!("sts.{region}.amazonaws.com")),
    });
    if let Some(value) = computed_header(computed, "X-Amz-Date") {
        headers.push(SubjectTokenHeader {
            key: "x-amz-date",
            value: Some(value.to_owned()),
        });
    }
    if let Some(value) = computed_header(computed, "X-Amz-Security-Token") {
        headers.push(SubjectTokenHeader {
            key: "x-amz-security-token",
            value: Some(value.to_owned()),
        });
    }
    headers.push(SubjectTokenHeader {
        key: TARGET_RESOURCE_HEADER,
        value: audience.map(str::to_owned),
    });
    headers
}

#[derive(serde::Serialize)]
struct SubjectTokenRequest {
    headers: Vec<SubjectTokenHeader>,
    method: &'static str,
    url: String,
}

#[derive(serde::Serialize)]
struct SubjectTokenHeader {
    key: &'static str,
    value: Option<String>,
}

fn computed_header<'computed>(
    computed: &'computed [(String, String)],
    name: &str,
) -> Option<&'computed str> {
    let (_, value) = computed
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))?;
    Some(value.as_str())
}
