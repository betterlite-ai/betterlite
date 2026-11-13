use betterlite_config::sections::compatible_image_credentials::Host;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_models::decisions::Json;

use super::error::CompatibleImageReplyError;
use crate::api::image_contracts::records::{ImageData, ImageHttpResponse, ImageResponse};
use crate::error::{ProviderError, ProviderErrorParts};

pub const MAX_REPLY_BYTES: usize = TransportLimits::DEFAULT.body_bytes;

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
    let Json::Object(fields) = parse_reply(reply)? else {
        return Err(CompatibleImageReplyError::NotObject);
    };
    let mut data = model_response.data.unwrap_or_default();
    match host {
        Host::Aiml => decode_aiml(&fields, &mut data)?,
        Host::CometApi => decode_cometapi(&fields, &mut data)?,
    }
    Ok(ImageResponse {
        data: Some(data),
        ..model_response
    })
}

// a body that is not JSON at all is B997's error with the reply's status and headers, before any shape is read
fn parse_reply(reply: &ImageHttpResponse) -> Result<Json, CompatibleImageReplyError> {
    serde_json::from_slice(&reply.body).map_err(|source| {
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
    })
}

// AIML picks exactly one layout: a list data field, else output.choices, else images
fn decode_aiml(
    fields: &indexmap::IndexMap<String, Json>,
    data: &mut Vec<ImageData>,
) -> Result<(), CompatibleImageReplyError> {
    if let Some(Json::List(items)) = fields.get("data") {
        for item in items {
            if let Some(image) = aiml_data_image(item)? {
                data.push(image);
            }
        }
        return Ok(());
    }
    if let Some(Json::Object(output)) = fields.get("output")
        && let Some(choices) = output.get("choices")
    {
        let Json::List(items) = choices else {
            return Err(CompatibleImageReplyError::List {
                field: "output.choices",
            });
        };
        for item in items {
            if let Some(image) = aiml_choice_image(item)? {
                data.push(image);
            }
        }
        return Ok(());
    }
    if let Some(images) = fields.get("images") {
        let Json::List(items) = images else {
            return Err(CompatibleImageReplyError::List { field: "images" });
        };
        for item in items {
            if let Some(image) = aiml_named_image(item)? {
                data.push(image);
            }
        }
    }
    Ok(())
}

// a present url wins even when null; the base64 branch takes b64_json before image_base64 and keeps revised_prompt
fn aiml_data_image(item: &Json) -> Result<Option<ImageData>, CompatibleImageReplyError> {
    let Json::Object(image) = item else {
        return Err(CompatibleImageReplyError::Object { field: "data" });
    };
    let revised_prompt = text_field(image.get("revised_prompt"), "revised_prompt")?;
    if image.contains_key("url") {
        let url = text_field(image.get("url"), "url")?;
        return Ok(Some(ImageData {
            b64_json: None,
            revised_prompt,
            url,
        }));
    }
    if image.contains_key("b64_json") || image.contains_key("image_base64") {
        let b64_json = text_field(image.get("b64_json"), "b64_json")?;
        let image_base64 = text_field(image.get("image_base64"), "image_base64")?;
        return Ok(Some(ImageData {
            b64_json: base64_or_fallback(b64_json, image_base64),
            revised_prompt,
            url: None,
        }));
    }
    Ok(None)
}

// a choice prefers a present image_base64 to url, and these two layouts never carry revised_prompt
fn aiml_choice_image(item: &Json) -> Result<Option<ImageData>, CompatibleImageReplyError> {
    let Json::Object(choice) = item else {
        return Err(CompatibleImageReplyError::Object {
            field: "output.choices",
        });
    };
    if choice.contains_key("image_base64") {
        let b64_json = text_field(choice.get("image_base64"), "image_base64")?;
        return Ok(Some(ImageData {
            b64_json,
            revised_prompt: None,
            url: None,
        }));
    }
    if choice.contains_key("url") {
        let url = text_field(choice.get("url"), "url")?;
        return Ok(Some(ImageData {
            b64_json: None,
            revised_prompt: None,
            url,
        }));
    }
    Ok(None)
}

// the images layout prefers a present url to image_base64
fn aiml_named_image(item: &Json) -> Result<Option<ImageData>, CompatibleImageReplyError> {
    let Json::Object(image) = item else {
        return Err(CompatibleImageReplyError::Object { field: "images" });
    };
    if image.contains_key("url") {
        let url = text_field(image.get("url"), "url")?;
        return Ok(Some(ImageData {
            b64_json: None,
            revised_prompt: None,
            url,
        }));
    }
    if image.contains_key("image_base64") {
        let b64_json = text_field(image.get("image_base64"), "image_base64")?;
        return Ok(Some(ImageData {
            b64_json,
            revised_prompt: None,
            url: None,
        }));
    }
    Ok(None)
}

// CometAPI reads any present data as a list and copies both nullable fields, dropping revised_prompt
fn decode_cometapi(
    fields: &indexmap::IndexMap<String, Json>,
    data: &mut Vec<ImageData>,
) -> Result<(), CompatibleImageReplyError> {
    let Some(value) = fields.get("data") else {
        return Ok(());
    };
    let Json::List(items) = value else {
        return Err(CompatibleImageReplyError::List { field: "data" });
    };
    for item in items {
        let Json::Object(image) = item else {
            return Err(CompatibleImageReplyError::Object { field: "data" });
        };
        let b64_json = text_field(image.get("b64_json"), "b64_json")?;
        let url = text_field(image.get("url"), "url")?;
        data.push(ImageData {
            b64_json,
            revised_prompt: None,
            url,
        });
    }
    Ok(())
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
