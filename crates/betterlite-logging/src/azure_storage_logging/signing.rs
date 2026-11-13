use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, Mac as _};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::Sha256;
use url::{ParseError, Url};

use super::transport::StorageRequest;

const HTTP_DATE: &str = "%a, %d %b %Y %H:%M:%S GMT";

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("the Azure storage request URL is not valid")]
    Url(#[source] ParseError),
    #[error("the Azure storage account key is not valid base64")]
    Key(#[source] base64::DecodeError),
    #[error("the Azure storage account key has an invalid length")]
    KeyLength(#[source] hmac::digest::InvalidLength),
}

// x-ms-date carries the clock, since a fixed date makes a signature reproducible in a test
#[must_use]
pub fn http_date(now: Duration) -> String {
    let seconds = i64::try_from(now.as_secs()).unwrap_or(i64::MAX);
    chrono::DateTime::from_timestamp(seconds, 0)
        .map_or_else(String::new, |moment| moment.format(HTTP_DATE).to_string())
}

fn canonicalized_headers(headers: &[(String, String)]) -> String {
    let mut lowercased: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _value)| name.to_ascii_lowercase().starts_with("x-ms-"))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();
    lowercased.sort_by(|left, right| left.0.cmp(&right.0));
    lowercased
        .into_iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect()
}

// an explicit query in the URL and the request's own parameters both belong to the signed resource
fn canonicalized_resource(account: &str, request: &StorageRequest) -> Result<String, SigningError> {
    let url = Url::parse(&request.url).map_err(SigningError::Url)?;
    let mut parameters: Vec<(String, String)> = url
        .query_pairs()
        .into_owned()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .collect();
    parameters.extend(
        request
            .query
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.clone())),
    );
    parameters.sort_by(|left, right| left.0.cmp(&right.0));
    let mut resource = format!("/{account}{}", url.path());
    for (name, value) in parameters {
        resource.push('\n');
        resource.push_str(&name);
        resource.push(':');
        resource.push_str(&value);
    }
    Ok(resource)
}

fn header_value<'headers>(headers: &'headers [(String, String)], wanted: &str) -> &'headers str {
    headers
        .iter()
        .find(|(name, _value)| name.eq_ignore_ascii_case(wanted))
        .map_or("", |(_name, value)| value.as_str())
}

fn content_length(request: &StorageRequest) -> String {
    if request.body.is_empty() {
        String::new()
    } else {
        request.body.len().to_string()
    }
}

fn canonical_string(request: &StorageRequest, account: &str) -> Result<String, SigningError> {
    let resource = canonicalized_resource(account, request)?;
    Ok(format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n\n{}\n{}\n{}\n{}\n{}\n{}{}",
        request.method.as_str(),
        header_value(&request.headers, "content-encoding"),
        header_value(&request.headers, "content-language"),
        content_length(request),
        header_value(&request.headers, "content-md5"),
        header_value(&request.headers, "content-type"),
        header_value(&request.headers, "if-modified-since"),
        header_value(&request.headers, "if-match"),
        header_value(&request.headers, "if-none-match"),
        header_value(&request.headers, "if-unmodified-since"),
        header_value(&request.headers, "range"),
        canonicalized_headers(&request.headers),
        resource,
    ))
}

#[must_use]
pub fn shared_key_authorization(
    account: &str,
    key: &SecretString,
    request: &StorageRequest,
) -> Result<String, SigningError> {
    let canonical = canonical_string(request, account)?;
    let decoded = BASE64
        .decode(key.expose_secret())
        .map_err(SigningError::Key)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&decoded).map_err(SigningError::KeyLength)?;
    mac.update(canonical.as_bytes());
    let signature = BASE64.encode(mac.finalize().into_bytes());
    Ok(format!("SharedKey {account}:{signature}"))
}
