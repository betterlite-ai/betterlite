use std::sync::Arc;
use std::time::SystemTime;

use secrecy::SecretString;

use super::Clock;
use crate::aws_settings::sources::Credentials;
use crate::signing::body::Body;
use crate::signing::endpoint::{RequestHeaders, request_headers};
use crate::signing::error::SigningError;
use crate::signing::request::AwsSignedRequest;

pub struct SigningInput<'source> {
    pub endpoint_url: &'source str,
    pub credentials: Option<&'source Credentials>,
    pub region: &'source str,
    pub extra_headers: &'source [(String, String)],
    pub api_key: Option<&'source SecretString>,
    pub configured_bearer: Option<&'source SecretString>,
    pub now: SystemTime,
}

pub fn sign_item(body: &[u8], input: &SigningInput<'_>) -> Result<AwsSignedRequest, SigningError> {
    let headers = merged_headers(input.extra_headers);
    request_headers(RequestHeaders {
        credentials: input.credentials,
        region: input.region,
        extra_headers: input.extra_headers,
        endpoint: input.endpoint_url,
        data: Body::Bytes(body),
        headers: &headers,
        api_key: input.api_key,
        configured_bearer: input.configured_bearer,
        supports_bearer_token: true,
        now: input.now,
    })
}

pub struct AsyncSigningInput {
    pub endpoint_url: String,
    pub body: Vec<u8>,
    pub credentials: Option<Credentials>,
    pub region: String,
    pub extra_headers: Vec<(String, String)>,
    pub api_key: Option<SecretString>,
    pub configured_bearer: Option<SecretString>,
    pub clock: Arc<dyn Clock>,
}

// LiteLLM's run_aws_signing always moves signing off the event loop, bearer or not
pub fn sign_owned(input: AsyncSigningInput) -> Result<AwsSignedRequest, SigningError> {
    let AsyncSigningInput {
        endpoint_url,
        body,
        credentials,
        region,
        extra_headers,
        api_key,
        configured_bearer,
        clock,
    } = input;
    sign_item(
        &body,
        &SigningInput {
            endpoint_url: &endpoint_url,
            credentials: credentials.as_ref(),
            region: &region,
            extra_headers: &extra_headers,
            api_key: api_key.as_ref(),
            configured_bearer: configured_bearer.as_ref(),
            now: clock.now(),
        },
    )
}

// the merged headers start as Content-Type and an exact extra-header name replaces its value
fn merged_headers(extra: &[(String, String)]) -> Vec<(String, Option<String>)> {
    let mut headers = vec![(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    for (name, value) in extra {
        match headers.iter_mut().find(|(existing, _)| existing == name) {
            Some((_, stored)) => *stored = Some(value.clone()),
            None => headers.push((name.clone(), Some(value.clone()))),
        }
    }
    headers
}
