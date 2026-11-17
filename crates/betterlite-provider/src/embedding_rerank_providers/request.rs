use betterlite_openai::openai_base_config::{Headers, Object as BaseObject};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;

use super::error::FamilyError;

// the request is serialized and read back as the family body's own JSON types
pub(super) fn plain<T, U>(value: &T) -> Result<U, FamilyError>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let bytes = encode(value)?;
    serde_json::from_slice(&bytes).map_err(|source| FamilyError::Encode { source })
}

pub(super) fn encode<T>(body: &T) -> Result<Vec<u8>, FamilyError>
where
    T: Serialize,
{
    serde_json::to_vec(body).map_err(|source| FamilyError::Encode { source })
}

pub(super) fn url(family: &'static str, value: &str) -> Result<Url, FamilyError> {
    Url::parse(value).map_err(|source| FamilyError::Url {
        family,
        value: value.to_owned(),
        source,
    })
}

// a family leaf's header map holds JSON strings, and a value of any other shape is dropped
pub(super) fn text_headers(headers: BaseObject) -> Headers {
    let mut mapped = Headers::new();
    for (name, value) in headers {
        let Some(text) = value.as_str() else {
            continue;
        };
        mapped.insert(name, text.to_owned());
    }
    mapped
}

// the credential header is marked sensitive, so no Debug text of a prepared call prints it
pub(super) fn http_headers(headers: &Headers) -> Result<HeaderMap, FamilyError> {
    let mut map = HeaderMap::new();
    for (name, text) in headers {
        let header_name =
            HeaderName::try_from(name.as_str()).map_err(|source| FamilyError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::try_from(text.as_str()).map_err(|source| FamilyError::Header {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    if let Some(value) = map.get_mut(http::header::AUTHORIZATION) {
        value.set_sensitive(true);
    }
    Ok(map)
}
