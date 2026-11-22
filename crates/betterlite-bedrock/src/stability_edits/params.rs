use betterlite_models::decisions::Json;

use super::error::StabilityEditError;
use super::types::{EditOptions, EditValue};

pub const SUPPORTED_OPENAI_PARAMS: [&str; 4] = ["n", "size", "response_format", "mask"];

// The native Stability aspect-ratio table B1522 owns; a provider leaf cannot import betterlite-core
const SIZE_TO_ASPECT_RATIO: [(&str, &str); 5] = [
    ("1024x1024", "1:1"),
    ("1792x1024", "16:9"),
    ("1024x1792", "9:16"),
    ("512x512", "1:1"),
    ("256x256", "1:1"),
];

pub fn map_openai_params(
    image_edit_optional_params: &EditOptions,
    model: &str,
    drop_params: bool,
) -> Result<EditOptions, StabilityEditError> {
    let mut mapped = image_edit_optional_params.clone();
    for (name, value) in image_edit_optional_params {
        match name.as_str() {
            "size" => map_size(value, &mut mapped)?,
            "n" | "response_format" => {
                mapped.insert(format!("_{name}"), value.clone());
            }
            _ if drop_params || SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) => {}
            _ => {
                return Err(StabilityEditError::UnsupportedParameter {
                    parameter: name.clone(),
                    model: model.to_owned(),
                });
            }
        }
    }
    for removed in ["size", "n", "response_format"] {
        mapped.shift_remove(removed);
    }
    Ok(mapped)
}

// A list or object size is unhashable in LiteLLM's dict lookup, which raises TypeError
fn map_size(value: &EditValue, mapped: &mut EditOptions) -> Result<(), StabilityEditError> {
    match value {
        EditValue::List(_) | EditValue::Json(Json::List(_) | Json::Object(_)) => {
            return Err(StabilityEditError::Field {
                field: "size".to_owned(),
            });
        }
        EditValue::Json(Json::Text(size)) => {
            if let Some(ratio) = aspect_ratio(size) {
                mapped.insert(
                    "aspect_ratio".to_owned(),
                    EditValue::Json(Json::Text(ratio.to_owned())),
                );
            }
        }
        EditValue::Json(Json::Null | Json::Bool(_) | Json::Number(_)) | EditValue::Bytes(_) => {}
    }
    Ok(())
}

fn aspect_ratio(size: &str) -> Option<&'static str> {
    SIZE_TO_ASPECT_RATIO
        .iter()
        .find_map(|(key, ratio)| (*key == size).then_some(*ratio))
}
