use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use url::Url;

use super::call::PreparedImageVariation;
use super::error::ImageVariationSource;

pub fn outbound_request(
    prepared: &PreparedImageVariation,
) -> Result<OutboundRequest, ImageVariationSource> {
    let (headers, mut options) = header_options(prepared)?;
    let query = query_pairs(&mut options)?;
    let extra_body = take_extra(&mut options, "extra_body")?;
    let parts = form_parts(&options, &extra_body, prepared)?;
    Ok(OutboundRequest {
        method: Method::POST,
        url: variation_url(&prepared.client.base_url)?,
        query,
        headers,
        body: RequestBody::Multipart {
            boundary: prepared.boundary.clone(),
            parts,
        },
        deadline: prepared.deadline,
    })
}

fn header_options(
    prepared: &PreparedImageVariation,
) -> Result<(HeaderMap, IndexMap<String, Json>), ImageVariationSource> {
    let mut headers = HeaderMap::new();
    if let Some(key) = &prepared.client.api_key {
        let bearer = format!("Bearer {}", key.expose_secret());
        insert_header(&mut headers, "authorization", &bearer, true)?;
    }
    if let Some(organization) = &prepared.client.organization {
        insert_header(&mut headers, "openai-organization", organization, false)?;
    }
    let mut options = prepared.data.options.clone();
    for (name, value) in take_extra(&mut options, "extra_headers")? {
        let Json::Text(text) = value else {
            return Err(ImageVariationSource::Option {
                field: String::from("extra_headers"),
            });
        };
        insert_header(&mut headers, &name, &text, false)?;
    }
    Ok((headers, options))
}

fn form_parts(
    options: &IndexMap<String, Json>,
    extra_body: &IndexMap<String, Json>,
    prepared: &PreparedImageVariation,
) -> Result<Vec<MultipartPart>, ImageVariationSource> {
    let mut parts = Vec::new();
    for (name, value) in options.iter().chain(extra_body.iter()) {
        let Some(data) = form_value(name, value)? else {
            continue;
        };
        parts.push(MultipartPart {
            name: name.clone(),
            file_name: None,
            content_type: None,
            data: data.into_bytes(),
        });
    }
    parts.push(image_part(prepared));
    Ok(parts)
}

fn image_part(prepared: &PreparedImageVariation) -> MultipartPart {
    let image = &prepared.data.image;
    let file_name = image
        .filename
        .clone()
        .unwrap_or_else(|| String::from("upload"));
    // httpx guesses a missing part type from the file name, and sends none for an empty name
    let content_type = image.content_type.clone().or_else(|| {
        (!file_name.is_empty()).then(|| {
            mime_guess::from_path(&file_name)
                .first_raw()
                .map_or_else(|| String::from("application/octet-stream"), str::to_owned)
        })
    });
    MultipartPart {
        name: String::from("image"),
        file_name: Some(file_name),
        content_type,
        data: image.content.clone(),
    }
}

// the SDK drops a null or empty form value and writes the rest with Python's str text
fn form_value(name: &str, value: &Json) -> Result<Option<String>, ImageVariationSource> {
    match value {
        Json::Null => Ok(None),
        Json::Text(text) => Ok((!text.is_empty()).then(|| text.clone())),
        Json::Bool(flag) => Ok(Some(flag.to_string())),
        Json::Number(number) => Ok(Some(number.to_string())),
        Json::List(_) | Json::Object(_) => Err(ImageVariationSource::Option {
            field: name.to_owned(),
        }),
    }
}

fn variation_url(base_url: &str) -> Result<Url, ImageVariationSource> {
    let mut url = Url::parse(base_url).map_err(ImageVariationSource::Url)?;
    let path = format!("{}/images/variations", url.path().trim_end_matches('/'));
    url.set_path(&path);
    Ok(url)
}

fn query_pairs(
    options: &mut IndexMap<String, Json>,
) -> Result<Vec<(String, String)>, ImageVariationSource> {
    let mut pairs = Vec::new();
    for (name, value) in take_extra(options, "extra_query")? {
        let text = match value {
            Json::Text(text) => text,
            Json::Number(number) => number.to_string(),
            Json::Bool(flag) => flag.to_string(),
            Json::Null => String::new(),
            Json::List(_) | Json::Object(_) => {
                return Err(ImageVariationSource::Option {
                    field: String::from("extra_query"),
                });
            }
        };
        pairs.push((name, text));
    }
    Ok(pairs)
}

fn take_extra(
    options: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<IndexMap<String, Json>, ImageVariationSource> {
    match options.shift_remove(field) {
        None | Some(Json::Null) => Ok(IndexMap::new()),
        Some(Json::Object(fields)) => Ok(fields),
        Some(_) => Err(ImageVariationSource::Option {
            field: String::from(field),
        }),
    }
}

fn insert_header(
    headers: &mut HeaderMap,
    name: &str,
    value: &str,
    sensitive: bool,
) -> Result<(), ImageVariationSource> {
    let header_name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|source| ImageVariationSource::Header(http::Error::from(source)))?;
    let mut header_value = HeaderValue::from_str(value)
        .map_err(|source| ImageVariationSource::Header(http::Error::from(source)))?;
    header_value.set_sensitive(sensitive);
    headers.insert(header_name, header_value);
    Ok(())
}
