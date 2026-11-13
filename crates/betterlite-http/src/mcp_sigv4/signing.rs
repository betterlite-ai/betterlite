use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    SignableBody, SignableRequest, SigningInstructions, SigningParams, SigningSettings, sign,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use bytes::Bytes;
use http::{HeaderName, HeaderValue, Request, header};

use super::error::McpSigV4Error;

const CONTENT_SHA256: &str = "x-amz-content-sha256";

// botocore never signs these headers
const UNSIGNED_HEADERS: [&str; 14] = [
    "baggage",
    "connection",
    "expect",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "traceparent",
    "tracestate",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "user-agent",
    "x-amzn-trace-id",
];

fn signable_headers(request: &Request<Bytes>) -> Result<Vec<(String, String)>, McpSigV4Error> {
    let mut signable = Vec::new();
    for name in request.headers().keys() {
        if UNSIGNED_HEADERS.contains(&name.as_str()) {
            continue;
        }
        // httpx hands botocore one value per header, with repeated headers joined by a comma
        let mut values = Vec::new();
        for value in request.headers().get_all(name) {
            let text = value
                .to_str()
                .map_err(|source| McpSigV4Error::HeaderValue {
                    name: name.as_str().to_owned(),
                    source,
                })?;
            values.push(text);
        }
        signable.push((name.as_str().to_owned(), values.join(", ")));
    }
    Ok(signable)
}

fn precomputed_hash(request: &Request<Bytes>) -> Result<Option<String>, McpSigV4Error> {
    let Some(precomputed) = request.headers().get(CONTENT_SHA256) else {
        return Ok(None);
    };
    let checksum = precomputed
        .to_str()
        .map_err(|source| McpSigV4Error::HeaderValue {
            name: CONTENT_SHA256.to_owned(),
            source,
        })?;
    Ok(Some(checksum.to_owned()))
}

fn apply_instructions(
    request: &mut Request<Bytes>,
    instructions: SigningInstructions,
) -> Result<(), McpSigV4Error> {
    let (signed_headers, _query) = instructions.into_parts();
    for signed in signed_headers {
        let name = HeaderName::from_static(signed.name());
        let mut value = HeaderValue::from_str(signed.value()).map_err(|source| {
            McpSigV4Error::SignatureHeader {
                name: signed.name(),
                source,
            }
        })?;
        value.set_sensitive(signed.sensitive());
        request.headers_mut().insert(name, value);
    }
    Ok(())
}

fn signing_params<'params>(
    identity: &'params Identity,
    region_name: &'params str,
    service_name: &'params str,
    now: SystemTime,
) -> Result<SigningParams<'params>, McpSigV4Error> {
    let params = v4::SigningParams::builder()
        .identity(identity)
        .region(region_name)
        .name(service_name)
        .time(now)
        .settings(SigningSettings::default())
        .build()
        .map_err(|source| McpSigV4Error::SigningParams { source })?;
    Ok(SigningParams::from(params))
}

pub(super) fn sign_request(
    request: &mut Request<Bytes>,
    credentials: &Credentials,
    region_name: &str,
    service_name: &str,
    now: SystemTime,
) -> Result<(), McpSigV4Error> {
    if request.headers().contains_key(header::DATE) {
        return Err(McpSigV4Error::DateHeader);
    }
    request.headers_mut().remove(header::AUTHORIZATION);
    request.headers_mut().remove("x-amz-date");
    if credentials.session_token().is_some() {
        request.headers_mut().remove("x-amz-security-token");
    }
    let headers = signable_headers(request)?;
    let precomputed = precomputed_hash(request)?;
    let body = precomputed.map_or_else(
        || SignableBody::Bytes(request.body()),
        SignableBody::Precomputed,
    );
    let identity = Identity::from(credentials.clone());
    let params = signing_params(&identity, region_name, service_name, now)?;
    let uri = request.uri().to_string();
    let signable = SignableRequest::new(
        request.method().as_str(),
        uri.as_str(),
        headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        body,
    )
    .map_err(|source| McpSigV4Error::SignableRequest { source })?;
    let (instructions, _signature) = sign(signable, &params)
        .map_err(|source| McpSigV4Error::Sign { source })?
        .into_parts();
    apply_instructions(request, instructions)
}
