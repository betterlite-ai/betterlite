use std::time::Duration;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::controls::multipart::{
    DEFAULT_MAX_RECURSE_DEPTH, MultipartField, serialize_multipart_fields,
};
use betterlite_models::decisions::Json;
use betterlite_prompts::media_metadata::{FileBody, FileData, extract_file_metadata};
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use url::Url;

use super::call::PreparedImageVariation;
use super::error::ImageVariationSource;

// the SDK's create_variation body writes these keywords in this order; the rest arrive through extra_body
const FORM_PARAMS: [&str; 5] = ["model", "n", "response_format", "size", "user"];

pub fn outbound_request(
    prepared: &PreparedImageVariation,
) -> Result<OutboundRequest, ImageVariationSource> {
    let (headers, mut options) = header_options(prepared)?;
    let query = query_pairs(&mut options)?;
    let extra_body = take_extra(&mut options, "extra_body")?;
    let deadline = request_deadline(&mut options, prepared.deadline)?;
    reject_unknown(&options)?;
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
        deadline,
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
            return Err(ImageVariationSource::Option { field: name });
        };
        insert_header(&mut headers, &name, &text, false)?;
    }
    Ok((headers, options))
}

// the SDK serializes its multipart body and its extra_query with the same bracket expansion
fn form_parts(
    options: &IndexMap<String, Json>,
    extra_body: &IndexMap<String, Json>,
    prepared: &PreparedImageVariation,
) -> Result<Vec<MultipartPart>, ImageVariationSource> {
    let fields = serialize_multipart_fields(
        &json_object(form_fields(options, extra_body)),
        DEFAULT_MAX_RECURSE_DEPTH,
    )?;
    let mut parts: Vec<MultipartPart> = fields.into_iter().map(form_part).collect();
    parts.push(image_part(prepared));
    Ok(parts)
}

// extra_body overrides a keyword where it stands and lands its new keys after the keywords
fn form_fields(
    options: &IndexMap<String, Json>,
    extra_body: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut merged = IndexMap::new();
    for name in FORM_PARAMS {
        if let Some(value) = options.get(name) {
            merged.insert(String::from(name), value.clone());
        }
    }
    for (name, value) in extra_body {
        match merged.get_mut(name) {
            Some(existing) => *existing = value.clone(),
            None => {
                merged.insert(name.clone(), value.clone());
            }
        }
    }
    merged
}

fn json_object(fields: IndexMap<String, Json>) -> JsonValue {
    JsonValue::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name, json_value(&value)))
            .collect(),
    )
}

// the image variation records hold the shared decisions::Json, which the controls' serializer does not take
fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::Text(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), json_value(item)))
                .collect(),
        ),
    }
}

fn form_part(field: MultipartField) -> MultipartPart {
    MultipartPart {
        name: field.name,
        file_name: None,
        content_type: None,
        data: field.value.into_bytes(),
    }
}

fn image_part(prepared: &PreparedImageVariation) -> MultipartPart {
    let image = &prepared.data.image;
    let supplied = image
        .filename
        .clone()
        .unwrap_or_else(|| String::from("upload"));
    // httpx keeps the last path component of a file object's name, and a tuple's filename as given
    let name = if image.content_type.is_some() {
        supplied
    } else {
        uploaded_name(&supplied)
    };
    // httpx guesses a missing part type through Python's mimetypes table, and writes no name or type for ""
    let content_type = image
        .content_type
        .clone()
        .or_else(|| (!name.is_empty()).then(|| guessed_content_type(&name)));
    MultipartPart {
        name: String::from("image"),
        file_name: (!name.is_empty()).then_some(name),
        content_type,
        data: image.content.clone(),
    }
}

// Python's Path(value.name).name: the last component, ignoring trailing slashes and "." components
fn uploaded_name(file_name: &str) -> String {
    file_name
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .next_back()
        .unwrap_or_default()
        .to_owned()
}

fn guessed_content_type(file_name: &str) -> String {
    let found = FileData::Tuple2 {
        filename: Some(file_name.to_owned()),
        content: FileBody::Bytes(Vec::new()),
    };
    extract_file_metadata(&found).1
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
    let fields = take_extra(options, "extra_query")?;
    let serialized = serialize_multipart_fields(&json_object(fields), DEFAULT_MAX_RECURSE_DEPTH)?;
    Ok(serialized
        .into_iter()
        .map(|MultipartField { name, value }| (name, value))
        .collect())
}

// create_variation takes a timeout keyword that overrides the client timeout for this one call
fn request_deadline(
    options: &mut IndexMap<String, Json>,
    client_deadline: Duration,
) -> Result<Duration, ImageVariationSource> {
    match options.shift_remove("timeout") {
        None | Some(Json::Null) => Ok(client_deadline),
        Some(Json::Number(number)) => number
            .as_f64()
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
            .ok_or(ImageVariationSource::Option {
                field: String::from("timeout"),
            }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(ImageVariationSource::Option {
                field: String::from("timeout"),
            })
        }
    }
}

// every remaining key reaches create_variation as a keyword, so an unknown one is the SDK's TypeError
fn reject_unknown(options: &IndexMap<String, Json>) -> Result<(), ImageVariationSource> {
    for name in options.keys() {
        if !FORM_PARAMS.contains(&name.as_str()) {
            return Err(ImageVariationSource::Unsupported {
                field: name.clone(),
            });
        }
    }
    Ok(())
}

fn take_extra(
    options: &mut IndexMap<String, Json>,
    field: &'static str,
) -> Result<IndexMap<String, Json>, ImageVariationSource> {
    let Some(value) = options.shift_remove(field) else {
        return Ok(IndexMap::new());
    };
    if matches!(value, Json::Null) {
        return Ok(IndexMap::new());
    }
    let Json::Object(fields) = value else {
        return Err(ImageVariationSource::Option {
            field: String::from(field),
        });
    };
    Ok(fields)
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
