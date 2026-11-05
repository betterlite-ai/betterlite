use std::error::Error;
use std::time::SystemTime;

use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SigningInstructions, SigningParams, SigningSettings, sign,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use http::header::CONTENT_TYPE;
use http::{HeaderMap, HeaderName, HeaderValue};

use crate::aws::request_signing::{AwsCredentialsInfo, AwsRequestError};

const SERVICE_NAME: &str = "secretsmanager";
const JSON_CONTENT_TYPE: &str = "application/x-amz-json-1.1";

pub(super) fn signed_headers<ResolverError>(
    credentials_info: &AwsCredentialsInfo,
    endpoint_url: &str,
    action: &str,
    body: &[u8],
    now: SystemTime,
) -> Result<HeaderMap, AwsRequestError<ResolverError>>
where
    ResolverError: Error + 'static,
{
    let target = format!("{SERVICE_NAME}.{action}");
    let identity = Identity::from(credentials_info.credentials.clone());
    let params = SigningParams::from(
        v4::SigningParams::builder()
            .identity(&identity)
            .region(&credentials_info.aws_region_name)
            .name(SERVICE_NAME)
            .time(now)
            .settings(SigningSettings::default())
            .build()
            .map_err(AwsRequestError::SigningParams)?,
    );
    let unsigned_headers = [
        ("content-type", JSON_CONTENT_TYPE),
        ("x-amz-target", &target),
    ];
    let signable = SignableRequest::new(
        "POST",
        endpoint_url,
        unsigned_headers.into_iter(),
        SignableBody::Bytes(body),
    )
    .map_err(AwsRequestError::Signing)?;
    let (instructions, _signature) = sign(signable, &params)
        .map_err(AwsRequestError::Signing)?
        .into_parts();

    response_headers(&target, &instructions)
}

fn response_headers<ResolverError>(
    target: &str,
    instructions: &SigningInstructions,
) -> Result<HeaderMap, AwsRequestError<ResolverError>>
where
    ResolverError: Error + 'static,
{
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
    headers.insert(
        HeaderName::from_static("x-amz-target"),
        HeaderValue::from_str(target).map_err(AwsRequestError::HeaderValue)?,
    );
    for (name, value) in instructions.headers() {
        let mut header_value =
            HeaderValue::from_str(value).map_err(AwsRequestError::HeaderValue)?;
        header_value.set_sensitive(name != "x-amz-date");
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).map_err(AwsRequestError::HeaderName)?,
            header_value,
        );
    }
    Ok(headers)
}
