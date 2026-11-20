use http::header::{HeaderName, HeaderValue};
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
            std::str::from_utf8(bytes).map_err(UploadError::Decode)?;
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
