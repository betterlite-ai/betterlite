use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SigningError as AwsSigningError, SigningParams, SigningSettings,
    sign,
};
use aws_sigv4::sign::v4;
use aws_sigv4::sign::v4::signing_params::BuildError;
use aws_smithy_runtime_api::client::identity::Identity;
use http::header::{InvalidHeaderValue, ToStrError};
use http::{HeaderName, HeaderValue, Request};

pub const SQS_SERVICE_NAME: &str = "sqs";

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("the SQS request header {name} is not valid text")]
    Header {
        name: String,
        #[source]
        source: ToStrError,
    },
    #[error("the SQS request could not be prepared for signing")]
    Signable(#[source] AwsSigningError),
    #[error("the SigV4 signing parameters could not be built")]
    Params(#[source] BuildError),
    #[error("the SQS request could not be signed")]
    Sign(#[source] AwsSigningError),
    #[error("the signed header {name} cannot be carried by the SQS request")]
    SignedHeader {
        name: &'static str,
        #[source]
        source: InvalidHeaderValue,
    },
}

fn signable_headers(request: &Request<Vec<u8>>) -> Result<Vec<(String, String)>, SigningError> {
    let mut headers = Vec::new();
    for name in request.headers().keys() {
        let mut values = Vec::new();
        for value in request.headers().get_all(name) {
            let text = value.to_str().map_err(|source| SigningError::Header {
                name: name.as_str().to_owned(),
                source,
            })?;
            values.push(text);
        }
        headers.push((name.as_str().to_owned(), values.join(", ")));
    }
    Ok(headers)
}

pub fn sign_request(
    request: &mut Request<Vec<u8>>,
    credentials: &Credentials,
    region_name: &str,
    now: SystemTime,
) -> Result<(), SigningError> {
    let headers = signable_headers(request)?;
    let identity = Identity::from(credentials.clone());
    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region_name)
        .name(SQS_SERVICE_NAME)
        .time(now)
        .settings(SigningSettings::default())
        .build()
        .map_err(SigningError::Params)?;
    let uri = request.uri().to_string();
    let signable = SignableRequest::new(
        request.method().as_str(),
        uri.as_str(),
        headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        SignableBody::Bytes(request.body()),
    )
    .map_err(SigningError::Signable)?;
    let (instructions, _signature) = sign(signable, &SigningParams::from(params))
        .map_err(SigningError::Sign)?
        .into_parts();
    let (signed_headers, _query) = instructions.into_parts();
    for signed in signed_headers {
        let name = HeaderName::from_static(signed.name());
        let mut value =
            HeaderValue::from_str(signed.value()).map_err(|source| SigningError::SignedHeader {
                name: signed.name(),
                source,
            })?;
        value.set_sensitive(signed.sensitive());
        request.headers_mut().insert(name, value);
    }
    Ok(())
}

mod tests;
