use betterlite_openai::openai_base_config::{Headers, Object as BaseObject};
use http::header::AUTHORIZATION;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use url::Url;

use super::Common;
use super::error::FamilyError;
use super::{Call, Decode};

// every family builds one prepared call the same way; only its named target, headers and body differ
pub(super) fn call<T>(
    common: &Common<'_>,
    target: (&'static str, &str),
    headers: &Headers,
    body: &T,
    decode: Decode,
) -> Result<Call, FamilyError>
where
    T: Serialize,
{
    let (family, endpoint) = target;
    Ok(Call {
        url: url(family, endpoint)?,
        headers: http_headers(headers)?,
        body: serde_json::to_vec(body)?,
        timeout: common.timeout,
        decode,
    })
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
    headers
        .into_iter()
        .filter_map(|(name, value)| Some((name, value.as_str()?.to_owned())))
        .collect()
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
    if let Some(value) = map.get_mut(AUTHORIZATION) {
        value.set_sensitive(true);
    }
    Ok(map)
}
