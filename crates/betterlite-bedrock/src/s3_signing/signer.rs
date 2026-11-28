use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_aws_settings::AwsS3Settings;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::s3_transport::signing::S3_SERVICE;
use betterlite_sigv4::sign::{Signable, SigningMode, sign_request};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};

use super::S3SigningError;
use crate::aws_settings::sources::Credentials;
use crate::aws_settings::{pop_aws_auth_params, s3_static_key_pair};
use crate::credentials::error::CredentialError;
use crate::signing::error::SigningError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::{RegionInputs, auth_settings, resolve_region};

const CACHE_CONTROL: &str = "private, immutable, max-age=31536000, s-maxage=0";
const CONTENT_SHA256: &str = "x-amz-content-sha256";
// botocore's header spelling and the order S3SigV4Auth adds them after the prepared headers
const SIGNED_HEADERS: [(&str, &str); 4] = [
    ("x-amz-date", "X-Amz-Date"),
    ("x-amz-security-token", "X-Amz-Security-Token"),
    (CONTENT_SHA256, "X-Amz-Content-SHA256"),
    ("authorization", "Authorization"),
];

// no Debug: optional_params carries the caller's AWS keys as raw JSON
#[derive(Clone, Copy)]
pub struct S3PutRequest<'source> {
    pub content: &'source str,
    pub api_base: &'source str,
    pub optional_params: &'source BTreeMap<String, Box<RawValue>>,
    pub encryption_key_id: Option<&'source SecretString>,
    pub region_inputs: RegionInputs<'source>,
    pub now: SystemTime,
}

fn secret_param(
    params: &BTreeMap<String, Box<RawValue>>,
    field: &'static str,
) -> Result<Option<SecretString>, S3SigningError> {
    params.get(field).map_or(Ok(None), |raw| {
        serde_json::from_str(raw.get()).map_err(|source| S3SigningError::Field {
            field,
            source: Some(source),
        })
    })
}

// the static s3_* pair signs as-is; otherwise the aws_* parameters go through the credential chain
async fn s3_credentials<F, Fut>(
    params: &BTreeMap<String, Box<RawValue>>,
    region: &str,
    resolve_credentials: F,
) -> Result<Credentials, S3SigningError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let s3 = AwsS3Settings {
        access_key_id: secret_param(params, "s3_access_key_id")?,
        secret_access_key: secret_param(params, "s3_secret_access_key")?,
    };
    if let Some((access_key_id, secret_access_key)) = s3_static_key_pair(&s3) {
        return Ok(Credentials {
            access_key_id,
            secret_access_key,
            session_token: None,
        });
    }
    let mut copy = params.clone();
    let auth = pop_aws_auth_params(&mut copy).map_err(SigningError::Auth)?;
    let settings = auth_settings(auth, region);
    resolve_credentials(&settings, region)
        .await
        .map_err(|source| S3SigningError::from(SigningError::Credential(source)))
}

fn upload_headers(
    content: &str,
    encryption_key_id: Option<&SecretString>,
) -> Vec<(String, String)> {
    let mut headers = vec![
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("Content-Language".to_owned(), "en".to_owned()),
        ("Cache-Control".to_owned(), CACHE_CONTROL.to_owned()),
    ];
    if let Some(key) = encryption_key_id.filter(|key| !key.expose_secret().is_empty()) {
        headers.push((
            "x-amz-server-side-encryption".to_owned(),
            "aws:kms".to_owned(),
        ));
        headers.push((
            "x-amz-server-side-encryption-aws-kms-key-id".to_owned(),
            key.expose_secret().to_owned(),
        ));
    }
    // requests sizes a str body by its UTF-8 bytes
    headers.push(("Content-Length".to_owned(), content.len().to_string()));
    headers
}

pub async fn sign_s3_request<F, Fut>(
    request: S3PutRequest<'_>,
    resolve_credentials: F,
) -> Result<AwsSignedRequest, S3SigningError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    let region = resolve_region(request.optional_params, Some(""), request.region_inputs)?;
    let credentials = s3_credentials(request.optional_params, &region, resolve_credentials).await?;
    let content_hash = hex::encode(Sha256::digest(request.content.as_bytes()));
    let mut headers = upload_headers(request.content, request.encryption_key_id);
    let mut signed_input = headers.clone();
    signed_input.push((CONTENT_SHA256.to_owned(), content_hash.clone()));
    // the region was resolved above, and the non-LLM region lookup returns a given region unchanged
    let mut computed = sign_request(&Signable {
        method: "PUT",
        url: request.api_base,
        headers: &signed_input,
        body: request.content.as_bytes(),
        service: S3_SERVICE,
        region: &region,
        credentials: &credentials,
        mode: SigningMode::S3SigV4Auth,
        now: request.now,
    })?;
    computed.push((CONTENT_SHA256.to_owned(), content_hash));
    for (name, botocore_name) in SIGNED_HEADERS {
        if let Some((_, value)) = computed
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
        {
            headers.push((botocore_name.to_owned(), value.clone()));
        }
    }
    Ok(AwsSignedRequest {
        method: "PUT".to_owned(),
        url: request.api_base.to_owned(),
        headers,
        body: request.content.as_bytes().to_vec(),
    })
}
