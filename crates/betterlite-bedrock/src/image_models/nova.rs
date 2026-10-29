use super::error::ImageModelsError;
use super::params::{Field, Params, pop_mapping};
use super::sizing::split_size;

pub const SUPPORTED_OPENAI_PARAMS: &[&str] = &["n", "size", "quality"];

pub fn map_openai_params(
    non_default_params: &Params,
    mut optional_params: Params,
) -> Result<Params, ImageModelsError> {
    if let Some(count) = non_default_params
        .get("n")
        .filter(|value| !matches!(value, Field::Null))
    {
        optional_params.insert("numberOfImages".to_owned(), count.clone());
    }
    if let Some(size) = non_default_params.get("size").and_then(Field::as_str) {
        let (width, height) = split_size(size)?;
        optional_params.insert("width".to_owned(), Field::Int(width));
        optional_params.insert("height".to_owned(), Field::Int(height));
    }
    if let Some(quality) = non_default_params.get("quality").and_then(Field::as_str) {
        match quality {
            "hd" | "premium" => {
                optional_params.insert("quality".to_owned(), Field::from("premium"));
            }
            "standard" => {
                optional_params.insert("quality".to_owned(), Field::from("standard"));
            }
            _ => {}
        }
    }
    Ok(optional_params)
}

pub fn transform_request_body(
    text: &str,
    mut optional_params: Params,
) -> Result<Params, ImageModelsError> {
    let task_type = optional_params
        .shift_remove("taskType")
        .unwrap_or_else(|| Field::from("TEXT_IMAGE"));
    let mut image_generation_config = pop_mapping(&mut optional_params, "imageGenerationConfig")?;
    optional_params.shift_remove("model_id");
    image_generation_config.extend(optional_params);

    let nested_key = match task_type.as_str() {
        Some("TEXT_IMAGE") => "textToImageParams",
        Some("COLOR_GUIDED_GENERATION") => "colorGuidedGenerationParams",
        Some("INPAINTING") => "inpaintingParams",
        _ => {
            return Err(ImageModelsError::TaskTypeUnsupported(
                task_type.python_str(),
            ));
        }
    };
    let nested_params = pop_mapping(&mut image_generation_config, nested_key)?;
    let mut merged = Params::new();
    merged.insert("text".to_owned(), Field::from(text));
    merged.extend(nested_params);

    Ok(build_body(
        task_type,
        nested_key,
        merged,
        image_generation_config,
    ))
}

fn build_body(
    task_type: Field,
    nested_key: &'static str,
    nested_params: Params,
    image_generation_config: Params,
) -> Params {
    let mut body = Params::new();
    body.insert(nested_key.to_owned(), Field::from(nested_params));
    body.insert("taskType".to_owned(), task_type);
    body.insert(
        "imageGenerationConfig".to_owned(),
        Field::from(image_generation_config),
    );
    body
}
