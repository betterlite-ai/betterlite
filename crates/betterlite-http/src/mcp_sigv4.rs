use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use aws_credential_types::{Credentials, provider::SharedCredentialsProvider};
use aws_smithy_async::{rt::sleep::SharedAsyncSleep, time::SharedTimeSource};
use aws_smithy_runtime_api::client::http::SharedHttpClient;
use aws_types::os_shim_internal::Fs;
use bytes::Bytes;
use http::Request;
use secrecy::{ExposeSecret as _, SecretString};

use self::cache::CredentialCache;
use self::credentials::PROVIDER_NAME;
use self::error::McpSigV4Error;

pub mod cache;
pub mod credentials;
pub mod error;
pub mod signing;
mod tests;

pub const DEFAULT_SERVICE_NAME: &str = "bedrock-agentcore";
pub const DEFAULT_REGION_NAME: &str = "us-east-1";

#[derive(Debug, Default)]
pub struct McpSigV4Settings {
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub aws_region_name: Option<String>,
    pub aws_service_name: Option<String>,
    pub aws_role_name: Option<String>,
    pub aws_session_name: Option<String>,
}

pub struct AwsSources {
    pub environment: HashMap<String, String>,
    pub filesystem: Fs,
    pub client: SharedHttpClient,
    pub clock: SharedTimeSource,
    pub sleep: SharedAsyncSleep,
    pub timeout: Duration,
}

fn explicit_credentials(settings: &McpSigV4Settings) -> Option<Credentials> {
    let (Some(aws_access_key_id), Some(aws_secret_access_key)) =
        (&settings.aws_access_key_id, &settings.aws_secret_access_key)
    else {
        return None;
    };
    Some(Credentials::new(
        aws_access_key_id.expose_secret(),
        aws_secret_access_key.expose_secret(),
        settings
            .aws_session_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        None,
        PROVIDER_NAME,
    ))
}

pub struct McpSigV4Auth {
    pub requires_request_body: bool,
    pub service_name: String,
    pub region_name: String,
    pub clock: SharedTimeSource,
    pub cache: CredentialCache,
}

impl McpSigV4Auth {
    // LiteLLM raises ImportError when botocore is missing; aws-sigv4 is linked at build time, so that error cannot occur here
    pub async fn new(
        settings: McpSigV4Settings,
        sources: AwsSources,
    ) -> Result<Self, McpSigV4Error> {
        let service_name = settings
            .aws_service_name
            .clone()
            .unwrap_or_else(|| DEFAULT_SERVICE_NAME.to_owned());
        let region_name = settings
            .aws_region_name
            .clone()
            .unwrap_or_else(|| DEFAULT_REGION_NAME.to_owned());
        let explicit = explicit_credentials(&settings);
        let provider = if let Some(role_arn) = &settings.aws_role_name {
            let temporary =
                credentials::assume_role(role_arn, explicit, &settings, &region_name, &sources)
                    .await?;
            SharedCredentialsProvider::new(temporary)
        } else if let Some(supplied) = explicit {
            SharedCredentialsProvider::new(supplied)
        } else {
            // LiteLLM raises ValueError here when botocore finds no credentials
            credentials::get_credentials(&sources)
                .await?
                .ok_or(McpSigV4Error::NoCredentials)?
        };
        Ok(Self {
            requires_request_body: true,
            service_name,
            region_name,
            cache: CredentialCache {
                clock: sources.clock.clone(),
                provider,
                current: Mutex::new(None),
            },
            clock: sources.clock,
        })
    }

    pub async fn auth_flow(
        &self,
        mut request: Request<Bytes>,
    ) -> Result<Request<Bytes>, McpSigV4Error> {
        let credentials = self.cache.get().await?;
        // every request header and the body go into the signature, as with botocore's AWSRequest
        signing::sign_request(
            &mut request,
            &credentials,
            &self.region_name,
            &self.service_name,
            self.clock.now(),
        )?;
        Ok(request)
    }
}
