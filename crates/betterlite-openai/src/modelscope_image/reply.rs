use serde_json::Value;

use crate::image_generation::reply::{ImageDatum, ImageGenerationReply, ImageUsage};
use crate::openai_base_config::Object;

use super::error::{ModelScopeImageError, error_class};

pub struct ImageReply<'reply> {
    pub status: u16,
    pub body: &'reply [u8],
}

pub fn decode_image_generation_reply(
    reply: ImageReply<'_>,
    created_seconds: i64,
) -> Result<ImageGenerationReply, ModelScopeImageError> {
    let parsed: Value =
        serde_json::from_slice(reply.body).map_err(|source| ModelScopeImageError::Decode {
            class: error_class(reply.status),
            source,
        })?;
    let Value::Object(fields) = parsed else {
        return Err(ModelScopeImageError::Field { field: "body" });
    };
    if let Some(error) = fields.get("error") {
        return Err(reply_error(error, reply.status));
    }
    let images = data_items(&fields)?;
    Ok(ImageGenerationReply {
        created: created_seconds,
        data: images,
        usage: ImageUsage::default(),
        background: None,
        quality: None,
        output_format: None,
        size: None,
    })
}

// the "error" key is read even when it is null, as LiteLLM's `"error" in reply` check does
fn reply_error(error: &Value, status: u16) -> ModelScopeImageError {
    let Value::Object(fields) = error else {
        return ModelScopeImageError::Field { field: "error" };
    };
    let message = match fields.get("message") {
        Some(Value::String(text)) => format!("ModelScope error: {text}"),
        _ => String::from("ModelScope error: the reply reports an error"),
    };
    ModelScopeImageError::ErrorBody {
        class: error_class(status),
        message,
    }
}

fn data_items(fields: &Object) -> Result<Vec<ImageDatum>, ModelScopeImageError> {
    match fields.get("data") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items.iter().map(data_item).collect(),
        Some(_) => Err(ModelScopeImageError::Field { field: "data" }),
    }
}

fn data_item(item: &Value) -> Result<ImageDatum, ModelScopeImageError> {
    let Value::Object(fields) = item else {
        return Err(ModelScopeImageError::Field { field: "data" });
    };
    Ok(ImageDatum {
        b64_json: optional_text(fields, "b64_json")?,
        url: optional_text(fields, "url")?,
        revised_prompt: optional_text(fields, "revised_prompt")?,
        provider_specific_fields: None,
    })
}

fn optional_text(
    fields: &Object,
    name: &'static str,
) -> Result<Option<String>, ModelScopeImageError> {
    match fields.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ModelScopeImageError::Field { field: name }),
    }
}
