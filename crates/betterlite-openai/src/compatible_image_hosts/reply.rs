use betterlite_config::sections::compatible_image_credentials::Host;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::CompatibleImageReplyError;
use crate::api::image_contracts::records::{ImageData, ImageHttpResponse, ImageResponse};
use crate::error::{ProviderError, ProviderErrorParts};

pub const MAX_REPLY_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

type Fields = IndexMap<String, Json>;
type Found = Option<(Option<String>, Option<String>)>;

#[derive(Clone, Copy)]
enum Layout {
    Data,
    Choice,
    Named,
    CometApi,
}

// neither host copies created, downloads a returned URL or runs post_call: only the data list changes
pub fn decode_compatible_image_reply(
    host: Host,
    reply: &ImageHttpResponse,
    model_response: ImageResponse,
) -> Result<ImageResponse, CompatibleImageReplyError> {
    if reply.body.len() > MAX_REPLY_BYTES {
        return Err(CompatibleImageReplyError::TooLarge {
            bytes: reply.body.len(),
            limit: MAX_REPLY_BYTES,
        });
    }
    let Ok(Json::Object(fields)) =
        serde_json::from_slice(&reply.body).map_err(|source| json_failure(reply, &source))
    else {
        return Err(CompatibleImageReplyError::NotObject);
    };
    let mut data = model_response.data.unwrap_or_default();
    match host {
        Host::Aiml => aiml(&fields, &mut data)?,
        Host::CometApi => cometapi(&fields, &mut data)?,
    }
    Ok(ImageResponse {
        data: Some(data),
        ..model_response
    })
}

// a body that is not JSON is B997's error, carrying the reply's own status and headers
fn json_failure(
    reply: &ImageHttpResponse,
    source: &serde_json::Error,
) -> CompatibleImageReplyError {
    let error = ProviderError::from(ProviderErrorParts {
        status_code: reply.status_code,
        message: format!("the image generation reply is not JSON: {source}"),
        headers: Some(
            reply
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        ),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    });
    CompatibleImageReplyError::Provider(Box::new(error))
}

// AIML picks exactly one layout: a list data field, else output.choices, else images
fn aiml(fields: &Fields, data: &mut Vec<ImageData>) -> Result<(), CompatibleImageReplyError> {
    if let Some(Json::List(items)) = fields.get("data") {
        return push("data", Layout::Data, items, data);
    }
    if let Some(Json::Object(output)) = fields.get("output")
        && let Some(choices) = output.get("choices")
    {
        let Json::List(items) = choices else {
            return Err(CompatibleImageReplyError::List {
                field: "output.choices",
            });
        };
        return push("output.choices", Layout::Choice, items, data);
    }
    let Some(images) = fields.get("images") else {
        return Ok(());
    };
    let Json::List(items) = images else {
        return Err(CompatibleImageReplyError::List { field: "images" });
    };
    push("images", Layout::Named, items, data)
}

// CometAPI reads any present data as a list and copies both nullable fields for every entry
fn cometapi(fields: &Fields, data: &mut Vec<ImageData>) -> Result<(), CompatibleImageReplyError> {
    let Some(value) = fields.get("data") else {
        return Ok(());
    };
    let Json::List(items) = value else {
        return Err(CompatibleImageReplyError::List { field: "data" });
    };
    push("data", Layout::CometApi, items, data)
}

fn push(
    field: &'static str,
    layout: Layout,
    items: &[Json],
    data: &mut Vec<ImageData>,
) -> Result<(), CompatibleImageReplyError> {
    for item in items {
        if let Some(image) = entry(field, layout, item)? {
            data.push(image);
        }
    }
    Ok(())
}

fn entry(
    field: &'static str,
    layout: Layout,
    item: &Json,
) -> Result<Option<ImageData>, CompatibleImageReplyError> {
    let Json::Object(image) = item else {
        return Err(CompatibleImageReplyError::Object { field });
    };
    let revised_prompt = match layout {
        Layout::Data => text(image.get("revised_prompt"), "revised_prompt")?,
        Layout::Choice | Layout::Named | Layout::CometApi => None,
    };
    let found = match layout {
        Layout::Data => data_fields(image)?,
        Layout::Choice => keyed(image, [("image_base64", false), ("url", true)])?,
        Layout::Named => keyed(image, [("url", true), ("image_base64", false)])?,
        Layout::CometApi => Some((
            text(image.get("url"), "url")?,
            text(image.get("b64_json"), "b64_json")?,
        )),
    };
    Ok(found.map(|(url, b64_json)| ImageData {
        b64_json,
        revised_prompt,
        url,
    }))
}

// a data entry prefers a present url, then b64_json before image_base64 with Python's truthy fallback
fn data_fields(image: &Fields) -> Result<Found, CompatibleImageReplyError> {
    if image.contains_key("url") {
        return Ok(Some((text(image.get("url"), "url")?, None)));
    }
    if !image.contains_key("b64_json") && !image.contains_key("image_base64") {
        return Ok(None);
    }
    let b64_json = text(image.get("b64_json"), "b64_json")?;
    let image_base64 = text(image.get("image_base64"), "image_base64")?;
    let chosen = match b64_json {
        Some(value) if !value.is_empty() => Some(value),
        _ => image_base64,
    };
    Ok(Some((None, chosen)))
}

// the first present key wins even when null, and the pair names which field that key fills
fn keyed(
    image: &Fields,
    keys: [(&'static str, bool); 2],
) -> Result<Found, CompatibleImageReplyError> {
    for (name, is_url) in keys {
        if image.contains_key(name) {
            let value = text(image.get(name), name)?;
            return Ok(Some(if is_url { (value, None) } else { (None, value) }));
        }
    }
    Ok(None)
}

fn text(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<String>, CompatibleImageReplyError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(CompatibleImageReplyError::Text { field })
        }
    }
}
