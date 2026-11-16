use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageFiles, ImageParams};
use serde_json::Number;

use super::error::StabilityEditError;
use super::types::{EditOptions, EditValue};
use crate::api::stability::generation::types::DEFAULT_OUTPUT_FORMAT;

// the declared type of each option Stability sends as a normal form field; output_format is absent
pub const EDIT_TEXT_FIELDS: [(&str, EditField); 17] = [
    ("negative_prompt", EditField::Text),
    ("aspect_ratio", EditField::Text),
    ("seed", EditField::Integer),
    ("mode", EditField::Text),
    ("strength", EditField::Number),
    ("style_preset", EditField::Text),
    ("left", EditField::Integer),
    ("bottom", EditField::Integer),
    ("right", EditField::Integer),
    ("top", EditField::Integer),
    ("creativity", EditField::Number),
    ("search_prompt", EditField::Text),
    ("grow_mask", EditField::Integer),
    ("select_prompt", EditField::Text),
    ("control_strength", EditField::Number),
    ("composition_fidelity", EditField::Number),
    ("change_strength", EditField::Number),
];

pub const EDIT_FILE_FIELDS: [&str; 3] = ["mask", "init_image", "style_image"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    Text,
    Integer,
    Number,
}

impl EditField {
    fn read(self, value: &Json) -> Option<Json> {
        match self {
            Self::Text => read_text(value).map(Json::Text),
            Self::Integer => read_integer(value).map(Json::Number),
            Self::Number => read_number(value).map(Json::Number),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StabilityImageEditRequest {
    pub data: ImageParams,
    pub files: ImageFiles,
}

// LiteLLM's transform_image_edit_request: output_format and prompt, then the allowlisted options
pub fn transform_image_edit_request(
    prompt: Option<&str>,
    image: Option<&[ImageFile]>,
    options: &EditOptions,
) -> Result<StabilityImageEditRequest, StabilityEditError> {
    let mut data = ImageParams::new();
    data.insert(
        "output_format".to_owned(),
        Json::Text(DEFAULT_OUTPUT_FORMAT.to_owned()),
    );
    if let Some(text) = prompt.filter(|given| !given.is_empty()) {
        data.insert("prompt".to_owned(), Json::Text(text.to_owned()));
    }
    let mut files = ImageFiles::new();
    if let Some(images) = image {
        files.push(("image".to_owned(), first_file("image", images)?));
    }
    apply_options(&mut data, &mut files, options)?;
    Ok(StabilityImageEditRequest { data, files })
}

// the allowlisted text fields and the three file parts, in the options' own order
fn apply_options(
    data: &mut ImageParams,
    files: &mut ImageFiles,
    options: &EditOptions,
) -> Result<(), StabilityEditError> {
    for (name, value) in options {
        if name.starts_with('_') || matches!(value, EditValue::Json(Json::Null)) {
            continue;
        }
        if let Some(field) = EDIT_FILE_FIELDS
            .iter()
            .copied()
            .find(|field| *field == name)
        {
            let EditValue::Files(parts) = value else {
                return Err(StabilityEditError::RequestField { field });
            };
            files.push((name.clone(), first_file(field, parts)?));
            continue;
        }
        if let Some((field, kind)) = EDIT_TEXT_FIELDS
            .iter()
            .copied()
            .find(|(field, _kind)| *field == name)
        {
            let EditValue::Json(json) = value else {
                return Err(StabilityEditError::RequestField { field });
            };
            let kept = kind
                .read(json)
                .ok_or(StabilityEditError::RequestField { field })?;
            data.insert(name.clone(), kept);
        }
    }
    Ok(())
}

fn first_file(field: &'static str, files: &[ImageFile]) -> Result<ImageFile, StabilityEditError> {
    let Some(first) = files.first() else {
        return Err(StabilityEditError::MissingFile { field });
    };
    Ok(first.clone())
}

fn read_text(value: &Json) -> Option<String> {
    let Json::Text(text) = value else {
        return None;
    };
    Some(text.clone())
}

fn read_integer(value: &Json) -> Option<Number> {
    let Json::Number(number) = value else {
        return None;
    };
    number.as_i64().map(Number::from)
}

fn read_number(value: &Json) -> Option<Number> {
    let Json::Number(number) = value else {
        return None;
    };
    Some(number.clone())
}
