use betterlite_openai::chat::ContentPart;

use crate::classifier_message_text::error::ClassifierTextError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    pub url: String,
}

pub fn inline_image_part(part: &ContentPart) -> Result<Option<InlineImage>, ClassifierTextError> {
    let Some(url) = image_url(part)? else {
        return Ok(None);
    };
    if !url.starts_with("data:") {
        return Ok(None);
    }
    Ok(Some(InlineImage { url }))
}

// a remote URL is caller-controlled and some provider adapters would fetch it from the proxy host
fn image_url(part: &ContentPart) -> Result<Option<String>, ClassifierTextError> {
    match part.kind.as_str() {
        "image_url" => url_part(part),
        "input_image" => Ok(part
            .fields
            .get("image_url")
            .and_then(|value| value.as_str())
            .and_then(|url| (!url.is_empty()).then(|| url.to_owned()))),
        "image" => Ok(anthropic_image_url(part)),
        _ => Ok(None),
    }
}

fn url_part(part: &ContentPart) -> Result<Option<String>, ClassifierTextError> {
    let Some(value) = part.fields.get("image_url") else {
        return Ok(None);
    };
    if let Some(url) = value.as_str() {
        return Ok((!url.is_empty()).then(|| url.to_owned()));
    }
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let Some(url) = object.get("url") else {
        return Ok(None);
    };
    if url.is_null() {
        return Ok(None);
    }
    url.as_str()
        .map_or(Err(ClassifierTextError::ImageUrlNotString), |text| {
            Ok((!text.is_empty()).then(|| text.to_owned()))
        })
}

fn anthropic_image_url(part: &ContentPart) -> Option<String> {
    match source_text(part, "type", "").as_str() {
        "base64" => base64_image_url(part),
        "url" => Some(source_text(part, "url", "")),
        _ => None,
    }
}

fn base64_image_url(part: &ContentPart) -> Option<String> {
    let media_type = source_text(part, "media_type", "image/jpeg");
    let data = source_text(part, "data", "");
    (!data.is_empty()).then(|| format!("data:{media_type};base64,{data}"))
}

fn source_text(part: &ContentPart, key: &str, default: &str) -> String {
    part.fields
        .get("source")
        .and_then(|value| value.as_object())
        .and_then(|source| source.get(key))
        .and_then(|value| value.as_str())
        .unwrap_or(default)
        .to_owned()
}
