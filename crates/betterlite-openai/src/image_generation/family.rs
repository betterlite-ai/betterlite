use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

pub const DALL_E_2_PARAMS: [&str; 5] = ["n", "response_format", "quality", "size", "user"];
pub const DALL_E_3_PARAMS: [&str; 6] = ["n", "response_format", "quality", "size", "user", "style"];
pub const GPT_IMAGE_PARAMS: [&str; 8] = [
    "background",
    "moderation",
    "n",
    "output_compression",
    "output_format",
    "quality",
    "size",
    "user",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFamily {
    DallE2,
    DallE3,
    GptImage,
}

impl ImageFamily {
    #[must_use]
    pub const fn supported_params(self) -> &'static [&'static str] {
        match self {
            Self::DallE2 => &DALL_E_2_PARAMS,
            Self::DallE3 => &DALL_E_3_PARAMS,
            Self::GptImage => &GPT_IMAGE_PARAMS,
        }
    }
}

#[must_use]
pub fn select_image_family(model: &str) -> ImageFamily {
    if model.starts_with("dall-e-2") || model.is_empty() {
        ImageFamily::DallE2
    } else if model.starts_with("dall-e-3") {
        ImageFamily::DallE3
    } else {
        ImageFamily::GptImage
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageParamError {
    #[error(
        "parameter {parameter} is not supported for model {model}; supported parameters are {supported:?}"
    )]
    Unsupported {
        parameter: String,
        model: String,
        supported: &'static [&'static str],
    },
}

pub fn map_openai_params(
    family: ImageFamily,
    model: &str,
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
    drop_params: bool,
) -> Result<(), ImageParamError> {
    let supported = family.supported_params();
    for (name, value) in non_default_params {
        if optional_params.contains_key(name) {
            continue;
        }
        if supported.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
            continue;
        }
        if !drop_params {
            return Err(ImageParamError::Unsupported {
                parameter: name.clone(),
                model: model.to_owned(),
                supported,
            });
        }
    }
    Ok(())
}
