use std::time::SystemTime;

use betterlite_sigv4::sign::{Signable, SigningMode, sign_request};

use crate::aws_settings::sources::Credentials;
use crate::signing::error::SigningError;
use crate::signing::headers::{merge_computed, prepare, without_computed};
use crate::signing::request::AwsSignedRequest;

pub const BEDROCK_SERVICE: &str = "bedrock";

// _sign_get_request builds a botocore AWSRequest with data=None and prepares it after signing
pub fn sign_get_request(
    credentials: &Credentials,
    url: &str,
    headers: &[(String, String)],
    region: &str,
    now: SystemTime,
) -> Result<AwsSignedRequest, SigningError> {
    let supplied = without_computed(headers.to_vec(), credentials.session_token.is_some());
    let computed = sign_request(&Signable {
        method: "GET",
        url,
        headers: &supplied,
        body: &[],
        service: BEDROCK_SERVICE,
        region,
        credentials,
        mode: SigningMode::SigV4Auth,
        now,
    })?;
    let merged = merge_computed(supplied, computed);
    Ok(AwsSignedRequest {
        method: "GET".to_owned(),
        url: url.to_owned(),
        headers: prepare(merged, 0, "GET"),
        body: Vec::new(),
    })
}
