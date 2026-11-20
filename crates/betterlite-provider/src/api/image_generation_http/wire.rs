use std::slice::from_ref;
use std::time::Duration;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use url::Url;
use uuid::Uuid;

use super::error::ImageGenerationError;

pub(super) fn outbound(
    url: &Url,
    headers: &ImageHeaders,
    data: &ImageParams,
    timeout: Duration,
    multipart: bool,
) -> Result<OutboundRequest, ImageGenerationError> {
    Ok(OutboundRequest {
        method: Method::POST,
        url: url.clone(),
        query: Vec::new(),
        headers: headers
            .iter()
            .map(|(name, value)| header(name, value))
            .collect::<Result<HttpHeaders, _>>()?,
        body: body(data, multipart)?,
        deadline: timeout,
    })
}

fn header(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), ImageGenerationError> {
    let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
        ImageGenerationError::HeaderName {
            name: name.to_owned(),
            source,
        }
    })?;
    let header_value =
        HeaderValue::from_str(value).map_err(|source| ImageGenerationError::HeaderValue {
            name: name.to_owned(),
            source,
        })?;
    Ok((header_name, header_value))
}

fn body(data: &ImageParams, multipart: bool) -> Result<RequestBody, ImageGenerationError> {
    if !multipart {
        return serde_json::to_vec(data)
            .map(RequestBody::Json)
            .map_err(ImageGenerationError::Encode);
    }
    let mut parts = form_parts(data)?;
    parts.push(dummy_file());
    Ok(RequestBody::Multipart {
        boundary: Uuid::new_v4().simple().to_string(),
        parts,
    })
}

fn form_parts(data: &ImageParams) -> Result<Vec<MultipartPart>, ImageGenerationError> {
    let mut parts = Vec::new();
    for (name, value) in data {
        push_field(&mut parts, name, value)?;
    }
    Ok(parts)
}

// httpx writes one field per list item, renders a primitive as its JSON spelling, and refuses an object
fn push_field(
    parts: &mut Vec<MultipartPart>,
    name: &str,
    value: &Json,
) -> Result<(), ImageGenerationError> {
    let items: &[Json] = match value {
        Json::List(items) => items.as_slice(),
        Json::Object(_) => {
            return Err(ImageGenerationError::FormValue {
                field: name.to_owned(),
            });
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => from_ref(value),
    };
    for item in items {
        let text = match item {
            Json::Text(text) => text.clone(),
            Json::Number(number) => number.to_string(),
            Json::Bool(setting) => setting.to_string(),
            Json::Null => String::new(),
            Json::List(_) | Json::Object(_) => {
                return Err(ImageGenerationError::FormValue {
                    field: name.to_owned(),
                });
            }
        };
        parts.push(MultipartPart {
            name: name.to_owned(),
            file_name: None,
            content_type: None,
            data: text.into_bytes(),
        });
    }
    Ok(())
}

// httpx's files={"none": ""}: one empty part called upload, its content type guessed from that name
fn dummy_file() -> MultipartPart {
    MultipartPart {
        name: "none".to_owned(),
        file_name: Some("upload".to_owned()),
        content_type: Some("application/octet-stream".to_owned()),
        data: Vec::new(),
    }
}
