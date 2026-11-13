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
        Host::Aiml => aiml_reply(&fields, &mut data)?,
        Host::CometApi => cometapi_reply(&fields, &mut data)?,
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
fn aiml_reply(fields: &Fields, data: &mut Vec<ImageData>) -> Result<(), CompatibleImageReplyError> {
    if let Some(Json::List(items)) = fields.get("data") {
        return push_entries(items, Layout::Data, data);
    }
    if let Some(Json::Object(output)) = fields.get("output")
        && let Some(choices) = output.get("choices")
    {
        let Json::List(items) = choices else {
            return Err(CompatibleImageReplyError::List {
                field: "output.choices",
            });
        };
        return push_entries(items, Layout::Choice, data);
    }
    if let Some(images) = fields.get("images") {
        let Json::List(items) = images else {
            return Err(CompatibleImageReplyError::List { field: "images" });
        };
        return push_entries(items, Layout::Named, data);
    }
    Ok(())
}

// CometAPI reads any present data as a list and copies both nullable fields for every entry
fn cometapi_reply(
    fields: &Fields,
    data: &mut Vec<ImageData>,
) -> Result<(), CompatibleImageReplyError> {
    let Some(value) = fields.get("data") else {
        return Ok(());
    };
    let Json::List(items) = value else {
        return Err(CompatibleImageReplyError::List { field: "data" });
    };
    push_entries(items, Layout::CometApi, data)
}

fn push_entries(
    items: &[Json],
    layout: Layout,
    data: &mut Vec<ImageData>,
) -> Result<(), CompatibleImageReplyError> {
    for item in items {
        if let Some(image) = entry_image(item, layout)? {
            data.push(image);
        }
    }
    Ok(())
}

fn entry_image(
    item: &Json,
    layout: Layout,
) -> Result<Option<ImageData>, CompatibleImageReplyError> {
    let Json::Object(image) = item else {
        return Err(CompatibleImageReplyError::Object {
            field: layout.field(),
        });
    };
    let revised_prompt = match layout {
        Layout::Data => text_field(image.get("revised_prompt"), "revised_prompt")?,
        Layout::Choice | Layout::Named | Layout::CometApi => None,
    };
    let found = match layout {
        Layout::Data => data_fields(image)?,
        Layout::Choice => keyed(image, [("image_base64", false), ("url", true)])?,
        Layout::Named => keyed(image, [("url", true), ("image_base64", false)])?,
        Layout::CometApi => cometapi_fields(image)?,
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
        let url = text_field(image.get("url"), "url")?;
        return Ok(Some((url, None)));
    }
    if !image.contains_key("b64_json") && !image.contains_key("image_base64") {
        return Ok(None);
    }
    let b64_json = text_field(image.get("b64_json"), "b64_json")?;
    let image_base64 = text_field(image.get("image_base64"), "image_base64")?;
    Ok(Some((None, base64_or_fallback(b64_json, image_base64))))
}

fn cometapi_fields(image: &Fields) -> Result<Found, CompatibleImageReplyError> {
    let b64_json = text_field(image.get("b64_json"), "b64_json")?;
    let url = text_field(image.get("url"), "url")?;
    Ok(Some((url, b64_json)))
}

// the first present key wins even when null, and the pair names which field that key fills
fn keyed(
    image: &Fields,
    keys: [(&'static str, bool); 2],
) -> Result<Found, CompatibleImageReplyError> {
    for (name, is_url) in keys {
        if image.contains_key(name) {
            let value = text_field(image.get(name), name)?;
            return Ok(Some(if is_url { (value, None) } else { (None, value) }));
        }
    }
    Ok(None)
}

fn text_field(
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

// Python's `b64_json or image_base64` takes the second value whenever the first is empty or absent
fn base64_or_fallback(b64_json: Option<String>, image_base64: Option<String>) -> Option<String> {
    match b64_json {
        Some(text) if !text.is_empty() => Some(text),
        _ => image_base64,
    }
}

impl Layout {
    const fn field(self) -> &'static str {
        match self {
            Self::Data | Self::CometApi => "data",
            Self::Choice => "output.choices",
            Self::Named => "images",
        }
    }
}
