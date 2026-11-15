use betterlite_models::decisions::Json;
use betterlite_models::image_cost::default_calculator::parse_image_dimensions;
use indexmap::IndexMap;
use serde_json::Number;

#[must_use]
pub fn map_gpt_image_size(size: &Json) -> Json {
    let Json::Text(text) = size else {
        return size.clone();
    };
    if text == "auto" {
        return size.clone();
    }
    let lowered = text.to_lowercase();
    // the model-cost parser also accepts Fal's -x- size separator; the GPT split rejects it
    if lowered.contains("-x-") {
        return size.clone();
    }
    let Ok((width, height)) = parse_image_dimensions(&lowered) else {
        return size.clone();
    };
    let mut mapped = IndexMap::new();
    mapped.insert("width".to_owned(), Json::Number(Number::from(width)));
    mapped.insert("height".to_owned(), Json::Number(Number::from(height)));
    Json::Object(mapped)
}
