use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use url::Url;

use super::call::PreparedImageCall;
use super::error::ImageGenerationSource;

mod tests;

pub fn outbound_request(
    prepared: &PreparedImageCall,
) -> Result<OutboundRequest, ImageGenerationSource> {
    let mut headers = default_headers(prepared)?;
    let mut body = prepared.wire_data.clone();
    for (name, value) in take_extra(&mut body, "extra_headers")? {
        let Json::Text(text) = value else {
            return Err(ImageGenerationSource::Extra {
                field: "extra_headers",
            });
        };
        insert_header(&mut headers, &name, &text, false)?;
    }
    let query = query_pairs(&mut body)?;
    let extra_body = take_extra(&mut body, "extra_body")?;
    body.extend(extra_body);
    let mut url = Url::parse(&prepared.api_base).map_err(ImageGenerationSource::Url)?;
    let path = format!("{}/images/generations", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query,
        headers,
        body: RequestBody::Json(serde_json::to_vec(&body).map_err(ImageGenerationSource::Encode)?),
        deadline: prepared.timeout,
    })
}

fn default_headers(prepared: &PreparedImageCall) -> Result<HeaderMap, ImageGenerationSource> {
    let key = prepared
        .api_key
        .as_ref()
        .ok_or(ImageGenerationSource::MissingApiKey)?;
    let mut headers = HeaderMap::new();
    insert_header(&mut headers, "content-type", "application/json", false)?;
    let bearer = format!("Bearer {}", key.expose_secret());
    insert_header(&mut headers, "authorization", &bearer, true)?;
    if let Some(organization) = &prepared.organization {
        insert_header(&mut headers, "openai-organization", organization, false)?;
    }
    Ok(headers)
}

fn query_pairs(
    body: &mut IndexMap<String, Json>,
) -> Result<Vec<(String, String)>, ImageGenerationSource> {
    let mut pairs = Vec::new();
    for (name, value) in take_extra(body, "extra_query")? {
        let text = match value {
            Json::Text(text) => text,
            Json::Number(number) => number.to_string(),
            Json::Bool(flag) => flag.to_string(),
            Json::Null => String::new(),
            Json::List(_) | Json::Object(_) => {
                return Err(ImageGenerationSource::Extra {
                    field: "extra_query",
                });
            }
        };
        pairs.push((name, text));
    }
    Ok(pairs)
}

fn take_extra(
    body: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<IndexMap<String, Json>, ImageGenerationSource> {
    match body.shift_remove(field) {
        None | Some(Json::Null) => Ok(IndexMap::new()),
        Some(Json::Object(fields)) => Ok(fields),
        Some(_) => Err(ImageGenerationSource::Extra { field }),
    }
}

fn insert_header(
    headers: &mut HeaderMap,
    name: &str,
    value: &str,
    sensitive: bool,
) -> Result<(), ImageGenerationSource> {
    let header_name = HeaderName::from_bytes(name.as_bytes())?;
    let mut header_value =
        HeaderValue::from_str(value).map_err(|source| ImageGenerationSource::RequestHeader {
            name: name.to_owned(),
            source,
        })?;
    header_value.set_sensitive(sensitive);
    headers.insert(header_name, header_value);
    Ok(())
}
