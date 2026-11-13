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

fn header(name: &str, value: &str) -> (String, String) {
    (name.to_owned(), value.to_owned())
}

pub fn storage_request(
    method: Method,
    url: String,
    auth: &StorageAuthorization<'_>,
    now: Duration,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> Result<StorageRequest, SigningError> {
    let mut headers = headers;
    headers.push(header("x-ms-version", STORAGE_API_VERSION));
    match auth {
        StorageAuthorization::Bearer(token) => headers.push(header(
            "Authorization",
            &format!("Bearer {}", token.expose_secret()),
        )),
        StorageAuthorization::SharedKey { account, key } => {
            headers.push(header("x-ms-date", &http_date(now)));
            let mut request = StorageRequest {
                method,
                url,
                query: Vec::new(),
                headers,
                body,
                deadline: STORAGE_TIMEOUT,
            };
            let value = shared_key_authorization(account, key, &request)?;
            request.headers.push(header("Authorization", &value));
            return Ok(request);
        }
    }
    Ok(StorageRequest {
        method,
        url,
        query: Vec::new(),
        headers,
        body,
        deadline: STORAGE_TIMEOUT,
    })
}

pub fn create_file_request(
    base_url: &str,
    auth: &StorageAuthorization<'_>,
    now: Duration,
) -> Result<StorageRequest, SigningError> {
    storage_request(
        Method::PUT,
        format!("{base_url}?resource=file"),
        auth,
        now,
        vec![header("Content-Length", "0")],
        Vec::new(),
    )
}

pub fn append_data_request(
    base_url: &str,
    auth: &StorageAuthorization<'_>,
    now: Duration,
    json_payload: &str,
) -> Result<StorageRequest, SigningError> {
    storage_request(
        Method::PATCH,
        format!("{base_url}?action=append&position=0"),
        auth,
        now,
        vec![header("Content-Type", "application/json")],
        json_payload.as_bytes().to_vec(),
    )
}

pub fn append_bytes_request(
    base_url: &str,
    auth: &StorageAuthorization<'_>,
    now: Duration,
    content: &[u8],
) -> Result<StorageRequest, SigningError> {
    storage_request(
        Method::PATCH,
        format!("{base_url}?action=append&position=0"),
        auth,
        now,
        vec![header("Content-Type", "application/json")],
        content.to_vec(),
    )
}

pub fn flush_data_request(
    base_url: &str,
    auth: &StorageAuthorization<'_>,
    now: Duration,
    position: usize,
) -> Result<StorageRequest, SigningError> {
    storage_request(
        Method::PATCH,
        format!("{base_url}?action=flush&position={position}"),
        auth,
        now,
        vec![header("Content-Length", "0")],
        Vec::new(),
    )
}

pub fn directory_exists_request(
    directory_url: &str,
    auth: &StorageAuthorization<'_>,
    now: Duration,
) -> Result<StorageRequest, SigningError> {
    storage_request(
        Method::HEAD,
        directory_url.to_owned(),
        auth,
        now,
        Vec::new(),
        Vec::new(),
    )
}

pub fn create_directory_request(
    directory_url: &str,
    auth: &StorageAuthorization<'_>,
    now: Duration,
) -> Result<StorageRequest, SigningError> {
    storage_request(
        Method::PUT,
        format!("{directory_url}?resource=directory"),
        auth,
        now,
        vec![header("Content-Length", "0")],
        Vec::new(),
    )
}
