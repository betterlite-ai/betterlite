use betterlite_openai::chat::ContentPart;

use crate::classifier_message_text::error::ClassifierTextError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    pub url: String,
}

#[must_use]
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
            .and_then(non_empty)),
        "image" => Ok(anthropic_image_url(part)),
        _ => Ok(None),
    }
}

fn url_part(part: &ContentPart) -> Result<Option<String>, ClassifierTextError> {
    let Some(value) = part.fields.get("image_url") else {
        return Ok(None);
    };
    if let Some(url) = value.as_str() {
        return Ok(non_empty(url));
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
    match url.as_str() {
        Some(url) => Ok(non_empty(url)),
        None => Err(ClassifierTextError::ImageUrlNotString),
    }
}

fn anthropic_image_url(part: &ContentPart) -> Option<String> {
    let source = part
        .fields
        .get("source")
        .and_then(|value| value.as_object())?;
    match source.get("type").and_then(|value| value.as_str())? {
        "base64" => {
            let media_type = source
                .get("media_type")
                .and_then(|value| value.as_str())
                .unwrap_or("image/jpeg");
            let data = source
                .get("data")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if data.is_empty() {
                return None;
            }
            Some(format!("data:{media_type};base64,{data}"))
        }
        "url" => Some(
            source
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_owned(),
        ),
        _ => None,
    }
}

fn non_empty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}
