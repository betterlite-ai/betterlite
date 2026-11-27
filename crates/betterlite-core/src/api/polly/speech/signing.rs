use std::time::SystemTime;

use betterlite_sigv4::credentials::Credentials;
use betterlite_sigv4::sign::{Signable, sign_request};
use indexmap::IndexMap;

use super::error::SpeechError;

pub const SERVICE: &str = "polly";
const CONTENT_TYPE_NAME: &str = "Content-Type";
const CONTENT_TYPE_VALUE: &str = "application/json";

pub struct SignablePost<'text> {
    pub url: &'text str,
    pub region: &'text str,
    pub body: &'text [u8],
    pub now: SystemTime,
    pub credentials: &'text Credentials,
}

// LiteLLM signs a POST over service=polly with only Content-Type set, and the bytes it signs are the body it sends
pub fn sign_post(request: &SignablePost<'_>) -> Result<IndexMap<String, String>, SpeechError> {
    let supplied = [(CONTENT_TYPE_NAME.to_owned(), CONTENT_TYPE_VALUE.to_owned())];
    let computed = sign_request(&Signable {
        method: "POST",
        url: request.url,
        headers: &supplied,
        body: request.body,
        service: SERVICE,
        region: request.region,
        credentials: request.credentials,
        now: request.now,
    })
    .map_err(SpeechError::Signing)?;
    let mut headers = IndexMap::new();
    for (name, value) in supplied.into_iter().chain(computed) {
        headers.insert(name, value);
    }
    Ok(headers)
}
