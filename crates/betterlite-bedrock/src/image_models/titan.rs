use super::error::ImageModelsError;
use super::params::{Field, Params, pop_mapping};
use super::sizing::split_size;

#[must_use]
pub const fn get_supported_openai_params() -> &'static [&'static str] {
    &["size", "n", "quality"]
}

pub fn map_openai_params(
    non_default_params: &Params,
    mut optional_params: Params,
) -> Result<Params, ImageModelsError> {
    let mut image_generation_config = Params::new();
    if let Some(size) = non_default_params.get("size").and_then(Field::as_str) {
        let (width, height) = split_size(size)?;
        image_generation_config.insert("width".to_owned(), Field::Int(width));
        image_generation_config.insert("height".to_owned(), Field::Int(height));
    }
    if let Some(count) = non_default_params
        .get("n")
        .filter(|value| !matches!(value, Field::Null))
    {
        image_generation_config.insert("numberOfImages".to_owned(), count.clone());
    }
    if let Some(quality) = non_default_params.get("quality").and_then(Field::as_str) {
        match quality {
            "hd" | "premium" | "high" => {
                image_generation_config.insert("quality".to_owned(), Field::from("premium"));
            }
            "standard" | "medium" | "low" => {
                image_generation_config.insert("quality".to_owned(), Field::from("standard"));
            }
            _ => {}
        }
    }
    if !image_generation_config.is_empty() {
        optional_params.insert(
            "imageGenerationConfig".to_owned(),
            Field::from(image_generation_config),
        );
    }
    Ok(optional_params)
}

pub fn transform_request_body(
    text: &str,
    mut optional_params: Params,
) -> Result<Params, ImageModelsError> {
    let mut image_generation_config = pop_mapping(&mut optional_params, "imageGenerationConfig")?;
    let negative_text = optional_params.shift_remove("negativeText");
    let mut text_to_image_params = Params::new();
    text_to_image_params.insert("text".to_owned(), Field::from(text));
    if let Some(value) = negative_text.filter(Field::truthy) {
        text_to_image_params.insert("negativeText".to_owned(), value);
    }
    let task_type = optional_params
        .shift_remove("taskType")
        .unwrap_or_else(|| Field::from("TEXT_IMAGE"));
    // The first pop above already removed the key, so this necessarily sees the {} default.
    let user_specified = pop_mapping(&mut optional_params, "imageGenerationConfig")?;
    image_generation_config.extend(user_specified);

    let mut body = Params::new();
    body.insert("taskType".to_owned(), task_type);
    body.insert(
        "textToImageParams".to_owned(),
        Field::from(text_to_image_params),
    );
    body.insert(
        "imageGenerationConfig".to_owned(),
        Field::from(image_generation_config),
    );
    Ok(body)
}
