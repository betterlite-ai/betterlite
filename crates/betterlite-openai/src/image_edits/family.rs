use crate::api::image_contracts::records::ImageParams;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 15] = [
    "image",
    "prompt",
    "background",
    "input_fidelity",
    "mask",
    "model",
    "n",
    "quality",
    "response_format",
    "size",
    "user",
    "extra_headers",
    "extra_query",
    "extra_body",
    "timeout",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEditFamily {
    DallE2,
    OpenAi,
}

impl ImageEditFamily {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::DallE2 => "dall_e_2",
            Self::OpenAi => "openai",
        }
    }

    // the multipart field name LiteLLM uses for each image; the mask is always "mask"
    #[must_use]
    pub const fn image_field(self) -> &'static str {
        match self {
            Self::DallE2 => "image",
            Self::OpenAi => "image[]",
        }
    }
}

// get_openai_image_edit_config lowercases the name and drops hyphens and underscores before comparing
#[must_use]
pub fn select_image_edit_family(model: &str) -> ImageEditFamily {
    let normalized = model.to_lowercase().replace('-', "").replace('_', "");
    if normalized == "dalle2" {
        ImageEditFamily::DallE2
    } else {
        ImageEditFamily::OpenAi
    }
}

// map_openai_params copies the options into a new dict and filters nothing, so a caller's map is untouched
#[must_use]
pub fn map_openai_params(options: &ImageParams) -> ImageParams {
    let mut mapped = ImageParams::new();
    mapped.extend(
        options
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    mapped
}
