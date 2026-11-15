use std::time::Duration;

use http::Method;
use secrecy::{ExposeSecret as _, SecretString};

use super::signing::{SigningError, http_date, shared_key_authorization};
use super::transport::{STORAGE_TIMEOUT, StorageRequest};

// LiteLLM's AZURE_STORAGE_MSFT_VERSION, sent on every create, append and flush call
pub const STORAGE_API_VERSION: &str = "2019-07-07";

pub enum StorageAuthorization<'secret> {
    Bearer(&'secret SecretString),
    SharedKey {
        account: &'secret str,
        key: &'secret SecretString,
    },
}

// LiteLLM's header sets: an empty create or flush says Content-Length 0, an append says Content-Type
pub fn storage_request(
    method: Method,
    url: String,
    auth: &StorageAuthorization<'_>,
    now: Duration,
    body: &[u8],
) -> Result<StorageRequest, SigningError> {
    let mut headers = vec![("x-ms-version".to_owned(), STORAGE_API_VERSION.to_owned())];
    if body.is_empty() && method != Method::HEAD {
        headers.push(("Content-Length".to_owned(), "0".to_owned()));
    }
    if !body.is_empty() {
        headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
    }
    let mut request = StorageRequest {
        method,
        url,
        query: Vec::new(),
        headers,
        body: body.to_vec(),
        deadline: STORAGE_TIMEOUT,
    };
    let value = match auth {
        StorageAuthorization::Bearer(token) => format!("Bearer {}", token.expose_secret()),
        StorageAuthorization::SharedKey { account, key } => {
            request
                .headers
                .push(("x-ms-date".to_owned(), http_date(now)));
            shared_key_authorization(account, key, &request)?
        }
    };
    request.headers.push(("Authorization".to_owned(), value));
    Ok(request)
}
