use std::str::from_utf8;

use http::header::{CONTENT_TYPE, HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use url::Url;

use crate::api::file_http_upload::error::UploadError;

use super::Mode;

pub(super) fn headers_from<'pair, H>(pairs: H) -> Result<HeaderMap, UploadError>
where
    H: Iterator<Item = (&'pair str, &'pair str)>,
{
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| UploadError::HeaderName {
                name: name.to_owned(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| UploadError::HeaderValue {
                name: name.to_owned(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

pub(super) fn request_url(text: &str) -> Result<Url, UploadError> {
    Url::parse(text).map_err(|source| UploadError::RequestUrl {
        url: text.to_owned(),
        source,
    })
}

// The sync handler decodes a bytes body before sending it, so invalid UTF-8 fails before HTTP
pub(super) fn traditional_body(mode: Mode, bytes: &[u8]) -> Result<Vec<u8>, UploadError> {
    match mode {
        Mode::Sync => {
            from_utf8(bytes).map_err(UploadError::Decode)?;
            Ok(bytes.to_vec())
        }
        Mode::Asynchronous => Ok(bytes.to_vec()),
    }
}

// getattr(client, upload_method.lower()): the method name is uppercased back to the wire token
pub(super) fn upload_method(method: &str) -> Result<Method, UploadError> {
    let token = method.to_ascii_uppercase();
    Method::from_bytes(token.as_bytes()).map_err(|_source| UploadError::Method {
        method: method.to_owned(),
    })
}

// The async aiohttp transport labels a bytes body with no content type application/octet-stream
pub(super) fn default_content_type(mut headers: HeaderMap, mode: Mode) -> HeaderMap {
    if mode == Mode::Asynchronous {
        headers
            .entry(CONTENT_TYPE)
            .or_insert(HeaderValue::from_static("application/octet-stream"));
    }
    headers
}

// A media upload carries the configured content type, or the octet-stream default when none is given
pub(super) fn media_headers(
    mut headers: HeaderMap,
    content_type: Option<&str>,
) -> Result<HeaderMap, UploadError> {
    let media_type = content_type
        .filter(|value| !value.is_empty())
        .unwrap_or("application/octet-stream");
    let value = HeaderValue::from_str(media_type).map_err(|source| UploadError::HeaderValue {
        name: String::from("content-type"),
        source,
    })?;
    headers.insert(CONTENT_TYPE, value);
    Ok(headers)
}
