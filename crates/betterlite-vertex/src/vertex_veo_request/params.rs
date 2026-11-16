use betterlite_bedrock::image_models::sizing::parse_python_int;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_prompts::arguments::json::{Json, Object};

use super::image::ImageInput;

// _OPENAI_VIDEO_SIZE_TO_ASPECT_RATIO and _OPENAI_VIDEO_SIZE_TO_RESOLUTION, in LiteLLM's map order
pub const SIZE_TO_ASPECT_RATIO: [(&str, &str); 4] = [
    ("1280x720", "16:9"),
    ("1920x1080", "16:9"),
    ("720x1280", "9:16"),
    ("1080x1920", "9:16"),
];

pub const SIZE_TO_RESOLUTION: [(&str, &str); 4] = [
    ("1280x720", "720p"),
    ("1920x1080", "1080p"),
    ("720x1280", "720p"),
    ("1080x1920", "1080p"),
];

// VideoCreateOptionalRequestParams: every key the mapper reads, with presence kept apart from null
pub struct MapInput<'media> {
    pub model: &'media str,
    pub input_reference: Option<ImageInput<'media>>,
    pub image: Option<ImageInput<'media>>,
    pub parameters: Option<Object>,
    pub resolution: Option<Json>,
    pub seconds: Option<String>,
    pub size: Option<String>,
}

pub struct MappedParams<'media> {
    pub image: Option<ImageInput<'media>>,
    pub parameters: Option<Object>,
    pub resolution: Option<Json>,
    pub aspect_ratio: Option<String>,
    pub duration_seconds: Option<i64>,
}

// _convert_size_to_aspect_ratio: an empty size maps nothing, and any unknown nonempty size is 16:9
#[must_use]
pub fn size_aspect_ratio(size: &str) -> Option<&'static str> {
    if size.is_empty() {
        return None;
    }
    Some(
        SIZE_TO_ASPECT_RATIO
            .iter()
            .find_map(|(openai, ratio)| (*openai == size).then_some(*ratio))
            .unwrap_or("16:9"),
    )
}

#[must_use]
pub fn size_resolution(size: &str) -> Option<&'static str> {
    SIZE_TO_RESOLUTION
        .iter()
        .find_map(|(openai, resolution)| (*openai == size).then_some(*resolution))
}

// _supports_resolution_inference: only a catalog entry with a non-null 1080p per-second rate
#[must_use]
pub fn supports_resolution_inference(model: &str, lookup: &dyn PriceLookup) -> bool {
    let key = if model.starts_with("vertex_ai/") {
        model.to_owned()
    } else {
        format!("vertex_ai/{model}")
    };
    let Some(record) = lookup.raw_cost_map_entry(&key) else {
        return false;
    };
    record
        .get("output_cost_per_second_1080p")
        .is_some_and(|rate| !matches!(rate, Scalar::Null))
}

// map_openai_params: input_reference becomes image before image does, and a failed seconds parse drops it
#[must_use]
pub fn map_openai_params<'media>(
    input: MapInput<'media>,
    lookup: &dyn PriceLookup,
) -> MappedParams<'media> {
    let MapInput {
        model,
        input_reference,
        image,
        parameters,
        resolution,
        seconds,
        size,
    } = input;
    let image = input_reference.or(image);
    // a resolution the caller set, flat or nested in parameters, blocks the size inference
    let nested_resolution = parameters.as_ref().is_some_and(|nested| {
        nested
            .get("resolution")
            .is_some_and(|value| !matches!(value, Json::Null))
    });
    let inferable = size.is_some() && resolution.is_none() && !nested_resolution;
    let inferred = if inferable {
        size.as_deref()
            .and_then(|size| inferred_resolution(size, model, lookup))
    } else {
        None
    };
    let resolution = inferred.or(resolution);
    let aspect_ratio = size
        .as_deref()
        .and_then(size_aspect_ratio)
        .map(str::to_owned);
    let duration_seconds = seconds.and_then(|text| parse_python_int(&text).ok());
    MappedParams {
        image,
        parameters,
        resolution,
        aspect_ratio,
        duration_seconds,
    }
}

// size → resolution for a model whose catalog carries a 1080p per-second rate; an unknown size infers nothing
fn inferred_resolution(size: &str, model: &str, lookup: &dyn PriceLookup) -> Option<Json> {
    if !supports_resolution_inference(model, lookup) {
        return None;
    }
    size_resolution(size).map(|resolution| Json::Text(resolution.to_owned()))
}
