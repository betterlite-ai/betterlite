use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use url::Url;

use crate::api::file_http_upload::error::UploadError;

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
