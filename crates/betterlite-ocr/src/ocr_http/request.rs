use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_error::{OcrError, OcrHeaderError, OcrTransportError};
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use http::HeaderName;
use http::HeaderValue;
use http::Method;
use url::Url;
pub(super) fn outbound(
    request: &PreparedOcrRequest,
    url: &str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> Result<OutboundRequest, OcrError> {
    let parsed = Url::parse(url).map_err(|_source| OcrError::RequestField {
        path: "url".to_owned(),
    })?;
    Ok(OutboundRequest {
        method: Method::POST,
        url: parsed,
        query: Vec::new(),
        headers: header_map(with_content_type(headers))?,
        body: RequestBody::Json(body),
        deadline: request.connection.timeout,
    })
}
pub(super) fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|text| (name.to_string(), text.to_owned()))
        })
        .collect()
}
pub(super) fn transport_failure(error: TransportError) -> OcrError {
    if matches!(
        error,
        TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
    ) {
        return OcrError::Transport(OcrTransportError::Timeout);
    }
    OcrError::Transport(OcrTransportError::Failed(error))
}
fn header_map(headers: Vec<(String, String)>) -> Result<HeaderMap, OcrError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            OcrError::Headers(OcrHeaderError::Name {
                name: name.clone(),
                source,
            })
        })?;
        let header_value = HeaderValue::from_str(&value).map_err(|source| {
            OcrError::Headers(OcrHeaderError::Value {
                name: name.clone(),
                source,
            })
        })?;
        map.append(header_name, header_value);
    }
    Ok(map)
}
pub(super) fn with_content_type(mut headers: Vec<(String, String)>) -> Vec<(String, String)> {
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
    {
        headers.push(("content-type".to_owned(), "application/json".to_owned()));
    }
    headers
}
