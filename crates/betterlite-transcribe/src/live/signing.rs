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

#[derive(Clone, Copy)]
pub struct ResolverSigner<'deps> {
    pub(super) resolver: &'deps AwsCredentialResolver<'deps>,
    pub(super) now: SystemTime,
}

fn signed_get(
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

impl ResolverSigner<'_> {
    async fn credentials(&self, region: &str) -> Result<Credentials, ProbeError> {
        self.resolver
            .resolve_credentials(&AwsSettings::default(), Some(region))
            .await
            .map_err(|source| ProbeError::Credentials(Box::new(source)))
    }
}

#[async_trait]
impl AwsSigner for ResolverSigner<'_> {
    async fn sign_json_post(
        &self,
        region: &str,
        url: &str,
        body: &str,
        headers: &[(String, String)],
    ) -> Result<AwsSignedRequest, ProbeError> {
        let credentials = self.credentials(region).await?;
        sign_aws_json_post(JsonPost {
            service_name: TRANSCRIBE_SERVICE,
            region: Some(region),
            url,
            body,
            headers,
            credentials: &credentials,
            now: self.now,
        })
        .map_err(ProbeError::Signing)
    }

    async fn sign_s3_get(&self, region: &str, url: &str) -> Result<AwsSignedRequest, ProbeError> {
        let credentials = self.credentials(region).await?;
        signed_get(&credentials, region, url, self.now)
    }
}
