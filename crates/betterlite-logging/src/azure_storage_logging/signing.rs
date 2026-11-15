use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::digest::InvalidLength;
use hmac::{Hmac, KeyInit as _, Mac as _};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::Sha256;
use url::{ParseError, Url};

use super::transport::StorageRequest;

const HTTP_DATE: &str = "%a, %d %b %Y %H:%M:%S GMT";

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("the Azure storage request URL {url} is not valid")]
    Url {
        url: String,
        #[source]
        source: ParseError,
    },
    #[error("the Azure storage account key is not valid base64")]
    Key(#[source] base64::DecodeError),
    #[error("the Azure storage account key has an invalid length")]
    KeyLength(#[source] InvalidLength),
}

// x-ms-date carries the clock, so a fixed clock makes a signature reproducible in a test
#[must_use]
pub fn http_date(now: Duration) -> String {
    let seconds = i64::try_from(now.as_secs()).unwrap_or(i64::MAX);
    chrono::DateTime::from_timestamp(seconds, 0)
        .map_or_else(String::new, |moment| moment.format(HTTP_DATE).to_string())
}

fn header<'request>(request: &'request StorageRequest, name: &str) -> &'request str {
    request
        .headers
        .iter()
        .find(|(present, _value)| present.eq_ignore_ascii_case(name))
        .map_or("", |(_name, value)| value.as_str())
}

fn resource(account: &str, url: &Url) -> String {
    let mut resource = format!("/{account}{}", url.path());
    let mut parameters: Vec<(String, String)> = url
        .query_pairs()
        .into_owned()
        .map(|(name, value)| (name.to_ascii_lowercase(), value))
        .collect();
    parameters.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, value) in parameters {
        resource.push('\n');
        resource.push_str(&name);
        resource.push(':');
        resource.push_str(&value);
    }
    resource
}

// the canonical string the service documents; x-ms-date replaces the Date line, which stays empty
fn canonical(request: &StorageRequest, account: &str) -> Result<String, SigningError> {
    let url = Url::parse(&request.url).map_err(|source| SigningError::Url {
        url: request.url.clone(),
        source,
    })?;
    let length = if request.body.is_empty() {
        String::new()
    } else {
        request.body.len().to_string()
    };
    let signed_headers = format!(
        "x-ms-date:{}\nx-ms-version:{}\n",
        header(request, "x-ms-date"),
        header(request, "x-ms-version")
    );
    Ok(format!(
        "{}\n\n\n{length}\n\n{}\n\n\n\n\n\n\n{signed_headers}{}",
        request.method.as_str(),
        header(request, "content-type"),
        resource(account, &url),
    ))
}

pub fn shared_key_authorization(
    account: &str,
    key: &SecretString,
    request: &StorageRequest,
) -> Result<String, SigningError> {
    let text = canonical(request, account)?;
    let decoded = BASE64
        .decode(key.expose_secret())
        .map_err(SigningError::Key)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&decoded).map_err(SigningError::KeyLength)?;
    mac.update(text.as_bytes());
    Ok(format!(
        "SharedKey {account}:{}",
        BASE64.encode(mac.finalize().into_bytes())
    ))
}
