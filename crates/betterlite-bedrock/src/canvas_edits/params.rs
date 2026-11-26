use super::types::CanvasEditOptions;
use crate::image_models::sizing::parse_python_int;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 13] = [
    "n",
    "size",
    "response_format",
    "mask",
    "negativeText",
    "similarityStrength",
    "cfgScale",
    "seed",
    "quality",
    "taskType",
    "maskPrompt",
    "outPaintingMode",
    "imageGenerationConfig",
];

const PREMIUM: &str = "premium";
const B64_JSON: &str = "b64_json";

pub struct MappedOptions {
    pub options: CanvasEditOptions,
    pub ignored_response_format: Option<IgnoredResponseFormat>,
}

pub struct IgnoredResponseFormat(String);

impl IgnoredResponseFormat {
    #[must_use]
    pub fn message(&self) -> String {
        format!(
            "Nova Canvas image edit ignores response_format={} and returns base64 images",
            self.0
        )
    }
}

// LiteLLM copies the options, rewrites size, n, quality and response_format, then drops unknown keys.
#[must_use]
pub fn map_openai_params(options: &CanvasEditOptions, drop_params: bool) -> MappedOptions {
    let mut mapped = options.clone();
    if let Some(size) = mapped.size.take()
        && let Some((width, height)) = size_pair(&size)
    {
        mapped.width = Some(width);
        mapped.height = Some(height);
    }
    if let Some(count) = mapped.image_count.take() {
        mapped.number_of_images = Some(count);
    }
    if let Some(quality) = mapped.quality.take() {
        mapped.quality = Some(mapped_quality(quality));
    }
    let ignored_response_format = mapped
        .response_format
        .take()
        .filter(|text| text.as_str() != B64_JSON)
        .map(IgnoredResponseFormat);
    if drop_params {
        mapped
            .passthrough
            .retain(|name, _value| name.starts_with('_'));
    }
    MappedOptions {
        options: mapped,
        ignored_response_format,
    }
}

// A size that is not "<int>x<int>" leaves width and height unset and does not raise.
pub(super) fn size_pair(size: &str) -> Option<(i64, i64)> {
    let (width, height) = size.split_once('x')?;
    Some((
        parse_python_int(width).ok()?,
        parse_python_int(height).ok()?,
    ))
}

// hd and premium both mean premium; every other value is kept as given for the API to judge.
fn mapped_quality(quality: String) -> String {
    match quality.as_str() {
        "hd" | PREMIUM => PREMIUM.to_owned(),
        _ => quality,
    }
}
