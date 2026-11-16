use betterlite_bedrock::image_models::sizing::parse_python_int;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::vertex_veo_request::image::ImageInput;
use crate::vertex_veo_request::params::{size_aspect_ratio, size_resolution};

// LiteLLM tests `"input_reference" in optional_params`, so a null reference still hides the image option
pub enum Reference<'media> {
    Absent,
    Null,
    Image(ImageInput<'media>),
}

pub struct MapInput<'media> {
    pub input_reference: Reference<'media>,
    pub image: Option<ImageInput<'media>>,
    pub size: Option<String>,
    pub seconds: Option<String>,
    pub resolution: Option<Json>,
    pub options: Object,
}

pub struct MappedParams<'media> {
    pub image: Option<ImageInput<'media>>,
    pub options: Object,
}

// map_openai_params: four sizes fill aspectRatio and a 720p/1080p resolution, any other size fills 16:9 only
#[must_use]
pub fn map_openai_params(input: MapInput<'_>) -> MappedParams<'_> {
    let image = match input.input_reference {
        Reference::Image(chosen) => Some(chosen),
        Reference::Null => None,
        Reference::Absent => input.image,
    };
    let mut options = Object::new();
    if let Some(ratio) = input.size.as_deref().and_then(size_aspect_ratio) {
        options.insert("aspectRatio".to_owned(), Json::Text(ratio.to_owned()));
    }
    if !resolution_set(input.resolution.as_ref())
        && let Some(preset) = input.size.as_deref().and_then(size_resolution)
    {
        options.insert("resolution".to_owned(), Json::Text(preset.to_owned()));
    }
    if let Some(requested) = input.seconds.as_deref()
        && let Ok(duration) = parse_python_int(requested)
    {
        options.insert(
            "durationSeconds".to_owned(),
            Json::Number(Number::from(duration)),
        );
    }
    if let Some(given) = input.resolution
        && !options.contains_key("resolution")
    {
        options.insert("resolution".to_owned(), given);
    }
    for (name, value) in input.options {
        if !options.contains_key(&name) {
            options.insert(name, value);
        }
    }
    MappedParams { image, options }
}

// `if not video_create_optional_params.get("resolution")`: absent, null and an empty string all allow inference
const fn resolution_set(resolution: Option<&Json>) -> bool {
    match resolution {
        Some(Json::Text(text)) => !text.is_empty(),
        None | Some(Json::Null) => false,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => true,
    }
}
