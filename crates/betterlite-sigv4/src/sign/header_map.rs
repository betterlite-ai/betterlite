use std::time::SystemTime;

use http::Error as HttpError;
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, Request};

use crate::credentials::Credentials;
use crate::error::SigV4Error;
use crate::sign::{Signable, SigningMode, sign_request};

pub fn insert_signed(
    headers: &mut HeaderMap,
    signed: Vec<(String, String)>,
) -> Result<(), SigV4Error> {
    for (name, value) in signed {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| SigV4Error::SignedHeader {
                name: name.clone(),
                source: HttpError::from(source),
            })?;
        let mut header_value =
            HeaderValue::from_str(&value).map_err(|source| SigV4Error::SignedHeader {
                name: name.clone(),
                source: HttpError::from(source),
            })?;
        header_value.set_sensitive(!name.eq_ignore_ascii_case("x-amz-date"));
        headers.insert(header_name, header_value);
    }
    Ok(())
}

pub fn sign_in_place<Body>(
    request: &mut Request<Body>,
    credentials: &Credentials,
    service: &str,
    region: &str,
    now: SystemTime,
) -> Result<(), SigV4Error>
where
    Body: AsRef<[u8]>,
{
    request.headers_mut().remove(header::AUTHORIZATION);
    request.headers_mut().remove("x-amz-date");
    if credentials.session_token.is_some() {
        request.headers_mut().remove("x-amz-security-token");
    }
    let mut headers = Vec::new();
    for name in request.headers().keys() {
        let mut values = Vec::new();
        for value in request.headers().get_all(name) {
            let text = value.to_str().map_err(|source| SigV4Error::HeaderText {
                name: name.as_str().to_owned(),
                source,
            })?;
            values.push(text);
        }
        headers.push((name.as_str().to_owned(), values.join(", ")));
    }
    let uri = request.uri().to_string();
    let signed = sign_request(&Signable {
        method: request.method().as_str(),
        url: &uri,
        headers: &headers,
        body: request.body().as_ref(),
        service,
        region,
        credentials,
        mode: SigningMode::SigV4Auth,
        now,
    })?;
    insert_signed(request.headers_mut(), signed)
}

mod tests;
