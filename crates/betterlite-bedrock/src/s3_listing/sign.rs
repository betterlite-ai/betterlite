use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::s3_transport::signing::{
    Credentials as SigningCredentials, RequestToSign, sign_request,
};
use sha2::{Digest as _, Sha256};

use super::S3ListingError;
use super::target::S3RequestTarget;
use crate::aws_settings::s3_static_key_pair;
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::signing::target::auth_settings;

const CONTENT_SHA256: &str = "x-amz-content-sha256";
// botocore's header spelling and the order S3SigV4Auth adds them after the prepared headers
const SIGNED_HEADERS: [(&str, &str); 4] = [
    ("x-amz-date", "X-Amz-Date"),
    ("x-amz-security-token", "X-Amz-Security-Token"),
    (CONTENT_SHA256, "X-Amz-Content-SHA256"),
    ("authorization", "Authorization"),
];

// the s3_* static pair signs as-is; otherwise the aws_* parameters go through the credential chain
pub async fn s3_credentials<F, Fut>(
    target: &S3RequestTarget,
    resolve_credentials: F,
) -> Result<Credentials, S3ListingError>
where
    F: FnOnce(&AwsSettings, &str) -> Fut,
    Fut: Future<Output = Result<Credentials, CredentialError>>,
{
    if let Some((access_key_id, secret_access_key)) = s3_static_key_pair(&target.s3) {
        return Ok(Credentials {
            access_key_id,
            secret_access_key,
            session_token: None,
        });
    }
    let settings = auth_settings(target.auth.clone(), &target.aws_region_name);
    resolve_credentials(&settings, &target.aws_region_name)
        .await
        .map_err(S3ListingError::Credential)
}

#[must_use]
pub fn sign_without_body(
    method: &str,
    api_base: &str,
    target: &S3RequestTarget,
    credentials: &Credentials,
    now: SystemTime,
) -> Result<Vec<(String, String)>, S3ListingError> {
    let empty_body_hash = hex::encode(Sha256::digest(b""));
    let initial = [(CONTENT_SHA256.to_owned(), empty_body_hash.clone())];
    let mut computed = sign_request(
        &RequestToSign {
            method,
            url: api_base,
            headers: &initial,
            content_sha256: &empty_body_hash,
            region: &target.aws_region_name,
            now,
        },
        &SigningCredentials {
            access_key_id: credentials.access_key_id.clone(),
            secret_access_key: credentials.secret_access_key.clone(),
            session_token: credentials.session_token.clone(),
        },
    )?;
    computed.push((CONTENT_SHA256.to_owned(), empty_body_hash));
    let mut headers = Vec::with_capacity(SIGNED_HEADERS.len());
    for (name, botocore_name) in SIGNED_HEADERS {
        if let Some((_, value)) = computed.iter().find(|(header, _)| header == name) {
            headers.push((botocore_name.to_owned(), value.clone()));
        }
    }
    Ok(headers)
}
