use std::error::Error;
use std::time::SystemTime;

use betterlite_sigv4::credentials::Credentials;
use betterlite_sigv4::sign::header_map::insert_signed;
use betterlite_sigv4::sign::{Signable, SigningMode, sign_request};
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderName, HeaderValue};

use crate::aws::request_signing::{AwsCredentialsInfo, AwsRequestError};

const SERVICE_NAME: &str = "secretsmanager";
const JSON_CONTENT_TYPE: &str = "application/x-amz-json-1.1";

pub(super) fn signed_headers<ResolverError, TransportFailure>(
    credentials_info: &AwsCredentialsInfo,
    endpoint_url: &str,
    action: &str,
    body: &[u8],
    now: SystemTime,
) -> Result<HeaderMap, AwsRequestError<ResolverError, TransportFailure>>
where
    ResolverError: Error + 'static,
    TransportFailure: Error + 'static,
{
    let target = format!("{SERVICE_NAME}.{action}");
    let request_headers = [
        ("content-type".to_owned(), JSON_CONTENT_TYPE.to_owned()),
        ("x-amz-target".to_owned(), target.clone()),
    ];
    let credentials = Credentials::from(&credentials_info.credentials);
    let signed = sign_request(&Signable {
        method: "POST",
        url: endpoint_url,
        headers: &request_headers,
        body,
        service: SERVICE_NAME,
        region: &credentials_info.aws_region_name,
        credentials: &credentials,
        mode: SigningMode::SigV4Auth,
        now,
    })
    .map_err(AwsRequestError::Signing)?;
    response_headers(&target, signed)
}

fn response_headers<ResolverError, TransportFailure>(
    target: &str,
    signed: Vec<(String, String)>,
) -> Result<HeaderMap, AwsRequestError<ResolverError, TransportFailure>>
where
    ResolverError: Error + 'static,
    TransportFailure: Error + 'static,
{
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
    headers.insert(
        HeaderName::from_static("x-amz-target"),
        HeaderValue::from_str(target).map_err(AwsRequestError::HeaderValue)?,
    );
    insert_signed(&mut headers, signed).map_err(AwsRequestError::Signing)?;
    Ok(headers)
}
