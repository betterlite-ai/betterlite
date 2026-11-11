use std::time::SystemTime;

use async_trait::async_trait;
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::AwsCredentialResolver;
use betterlite_bedrock::signing::json_post::{JsonPost, sign_aws_json_post};
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::sigv4::{Signable, sign_request};
use betterlite_config::sections::bedrock_settings::AwsSettings;

use crate::live::ProbeError;

const TRANSCRIBE_SERVICE: &str = "transcribe";
const S3_SERVICE: &str = "s3";

#[async_trait]
pub trait AwsSigner: Send + Sync {
    async fn sign_json_post(
        &self,
        region: &str,
        url: &str,
        body: &str,
        headers: &[(String, String)],
    ) -> Result<AwsSignedRequest, ProbeError>;

    async fn sign_s3_get(&self, region: &str, url: &str) -> Result<AwsSignedRequest, ProbeError>;
}

#[async_trait]
pub trait CredentialSource: Send + Sync {
    async fn credentials(&self, region: &str) -> Result<Credentials, ProbeError>;
}

#[derive(Clone, Copy)]
pub struct ResolverCredentials<'deps>(pub(super) &'deps AwsCredentialResolver<'deps>);

#[async_trait]
impl CredentialSource for ResolverCredentials<'_> {
    // the resolver reaches AWS credential sources, which no unit test can stand in for
    #[cfg_attr(test, mutants::skip)]
    async fn credentials(&self, region: &str) -> Result<Credentials, ProbeError> {
        self.0
            .resolve_credentials(&AwsSettings::default(), Some(region))
            .await
            .map_err(|source| ProbeError::Credentials(Box::new(source)))
    }
}

// the clock is read per request, like LiteLLM's run_aws_signing
#[derive(Clone, Copy)]
pub struct ClockedSigner<C, K> {
    pub(super) credentials: C,
    pub(super) clock: K,
}

pub(super) fn signed_get(
    credentials: &Credentials,
    region: &str,
    url: &str,
    now: SystemTime,
) -> Result<AwsSignedRequest, ProbeError> {
    let headers = sign_request(&Signable {
        method: "GET",
        url,
        headers: &[],
        body: &[],
        service: S3_SERVICE,
        region,
        credentials,
        now,
    })
    .map_err(ProbeError::Signing)?;
    Ok(AwsSignedRequest {
        method: "GET".to_owned(),
        url: url.to_owned(),
        headers,
        body: Vec::new(),
    })
}

#[async_trait]
impl<C, K> AwsSigner for ClockedSigner<C, K>
where
    C: CredentialSource,
    K: Fn() -> SystemTime + Send + Sync,
{
    async fn sign_json_post(
        &self,
        region: &str,
        url: &str,
        body: &str,
        headers: &[(String, String)],
    ) -> Result<AwsSignedRequest, ProbeError> {
        let credentials = self.credentials.credentials(region).await?;
        sign_aws_json_post(JsonPost {
            service_name: TRANSCRIBE_SERVICE,
            region: Some(region),
            url,
            body,
            headers,
            credentials: &credentials,
            now: (self.clock)(),
        })
        .map_err(ProbeError::Signing)
    }

    async fn sign_s3_get(&self, region: &str, url: &str) -> Result<AwsSignedRequest, ProbeError> {
        let credentials = self.credentials.credentials(region).await?;
        signed_get(&credentials, region, url, (self.clock)())
    }
}
