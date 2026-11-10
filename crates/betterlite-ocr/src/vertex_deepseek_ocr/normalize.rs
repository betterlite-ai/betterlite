use serde::Deserialize;

use crate::json::{Json, Object};
use crate::ocr_config::decode::{OcrFieldKind, decode_value};
use crate::ocr_error::OcrError;
use crate::response::{OCRPage, OCRPageDimensions, OCRPageImage, OCRResponse, OCRUsageInfo};

const CONTENT_PATH: &str = "choices[0].message.content";

#[derive(Debug, Deserialize)]
pub(crate) struct DeepSeekOcrResponse {
    #[serde(default)]
    choices: Vec<DeepSeekChoice>,
    #[serde(default = "default_usage")]
    usage: Json,
}

#[derive(Debug, Deserialize)]
struct DeepSeekChoice {
    #[serde(default)]
    message: DeepSeekMessage,
}

#[derive(Debug, Default, Deserialize)]
struct DeepSeekMessage {
    content: Option<DeepSeekContent>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DeepSeekContent {
    Text(String),
    Object(Object),
}

#[derive(Debug, Default, Deserialize)]
#[serde(try_from = "Json")]
struct LaxIndex(i64);

#[derive(Debug, Deserialize)]
struct DeepSeekPage {
    #[serde(default)]
    index: LaxIndex,
    #[serde(default)]
    markdown: String,
    images: Option<Vec<OCRPageImage>>,
    dimensions: Option<OCRPageDimensions>,
    #[serde(flatten)]
    extra: Object,
}

impl TryFrom<Json> for LaxIndex {
    type Error = LaxIndexError;

    fn try_from(value: Json) -> Result<Self, Self::Error> {
        integer_from_json(&value).map(Self).ok_or(LaxIndexError)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("expected an integer in the i64 range")]
struct LaxIndexError;

pub(crate) fn normalize_response(
    model: &str,
    response: DeepSeekOcrResponse,
) -> Result<OCRResponse, OcrError> {
    let content = response
        .choices
        .into_iter()
        .next()
        .and_then(|choice| choice.message.content)
        .ok_or(OcrError::EmptyContent)?;
    let (data, fallback) = content_data(content)?;
    let has_pages = data.contains_key("pages");
    let decoded_pages = decode_pages(&data)?;
    let pages = if decoded_pages.is_empty() {
        vec![OCRPage {
            index: 0,
            markdown: fallback,
            images: None,
            dimensions: None,
            extra: Object::new(),
        }]
    } else {
        decoded_pages
    };
    Ok(OCRResponse {
        pages,
        model: response_model(&data, model)?,
        document_annotation: has_pages
            .then(|| data.get("document_annotation").cloned())
            .flatten(),
        usage_info: decode_usage(&data, &response.usage, has_pages)?,
        content: None,
        tables: decode_objects(&data, "tables")?,
        key_value_pairs: decode_objects(&data, "keyValuePairs")?,
        object: "ocr".to_owned(),
        extra: extra_fields(&data),
        provider_native_response: None,
    })
}

fn content_data(content: DeepSeekContent) -> Result<(Object, String), OcrError> {
    match content {
        DeepSeekContent::Text(text) if text.is_empty() => Err(OcrError::EmptyContent),
        DeepSeekContent::Text(text) => {
            let parsed = text
                .trim_start()
                .starts_with('{')
                .then(|| serde_json::from_str::<Object>(&text).ok())
                .flatten();
            Ok((parsed.unwrap_or_default(), text))
        }
        DeepSeekContent::Object(data) if data.is_empty() => Err(OcrError::EmptyContent),
        DeepSeekContent::Object(data) => {
            let fallback = if data.contains_key("pages") {
                String::new()
            } else {
                serde_json::to_string(&Json::Object(data.clone())).map_err(|_source| {
                    OcrError::ResponseField {
                        path: format!("{CONTENT_PATH}.content"),
                    }
                })?
            };
            Ok((data, fallback))
        }
    }
}

fn decode_pages(data: &Object) -> Result<Vec<OCRPage>, OcrError> {
    let Some(value) = data.get("pages") else {
        return Ok(Vec::new());
    };
    let Json::List(entries) = value else {
        return Err(OcrError::ResponseField {
            path: format!("{CONTENT_PATH}.pages"),
        });
    };
    entries
        .iter()
        .enumerate()
        .filter(|(_, page)| matches!(page, Json::Object(_)))
        .map(|(position, page)| decode_page(page, position))
        .collect()
}

fn decode_page(page: &Json, position: usize) -> Result<OCRPage, OcrError> {
    let prefix = format!("{CONTENT_PATH}.pages[{position}]");
    let decoded: DeepSeekPage = decode_value(page, &prefix, OcrFieldKind::Response)?;
    Ok(OCRPage {
        index: decoded.index.0,
        markdown: decoded.markdown,
        images: decoded.images,
        dimensions: decoded.dimensions,
        extra: decoded.extra,
    })
}

fn decode_usage(
    data: &Object,
    fallback: &Json,
    has_pages: bool,
) -> Result<Option<OCRUsageInfo>, OcrError> {
    let selected = data
        .get("usage_info")
        .or_else(|| (!has_pages).then_some(fallback));
    let Some(usage) = selected else {
        return Ok(None);
    };
    if !matches!(usage, Json::Object(_)) {
        return Ok(None);
    }
    decode_value(usage, "usage_info", OcrFieldKind::Response).map(Some)
}

fn decode_objects(data: &Object, name: &str) -> Result<Option<Vec<Object>>, OcrError> {
    let Some(value) = data.get(name).filter(|value| !matches!(value, Json::Null)) else {
        return Ok(None);
    };
    decode_value(
        value,
        &format!("{CONTENT_PATH}.{name}"),
        OcrFieldKind::Response,
    )
    .map(Some)
}

fn response_model(data: &Object, requested: &str) -> Result<String, OcrError> {
    match data.get("model") {
        Some(Json::Text(model)) => Ok(model.clone()),
        Some(_) => Err(OcrError::ResponseField {
            path: format!("{CONTENT_PATH}.model"),
        }),
        None => Ok(requested.to_owned()),
    }
}

fn extra_fields(data: &Object) -> Object {
    data.iter()
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "pages"
                    | "model"
                    | "document_annotation"
                    | "usage_info"
                    | "object"
                    | "content"
                    | "tables"
                    | "keyValuePairs"
                    | "provider_native_response"
            )
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn integer_from_json(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(integral)),
        Json::Text(text) => integer_from_text(text.trim()),
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

// LiteLLM's LaxI64 takes an integral float; the parse refuses anything outside the i64 range
fn integral(value: f64) -> Option<i64> {
    if !value.is_finite() || value.fract() != 0.0_f64 {
        return None;
    }
    format!("{value:.0}").parse().ok()
}

fn integer_from_text(text: &str) -> Option<i64> {
    let integer = match text.split_once('.') {
        Some((integer, fraction))
            if !fraction.is_empty() && fraction.bytes().all(|byte| byte == b'0') =>
        {
            integer
        }
        Some(_) => return None,
        None => text,
    };
    if integer.starts_with('_') || integer.ends_with('_') || integer.contains("__") {
        return None;
    }
    let digits = integer.strip_prefix(['+', '-']).unwrap_or(integer);
    if digits.is_empty()
        || digits.starts_with('_')
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    integer.replace('_', "").parse().ok()
}

fn default_usage() -> Json {
    Json::Object(Object::new())
}
