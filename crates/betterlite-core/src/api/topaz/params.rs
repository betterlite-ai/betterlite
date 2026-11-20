use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;

use super::DEFAULT_API_BASE;
use super::error::TopazError;

// TopazImageVariationConfig.get_supported_openai_params
pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["response_format", "size"];

// TopazImageVariationConfig.get_complete_url keeps every slash its argument ends with
#[must_use]
pub fn get_complete_url(api_base: Option<&str>) -> String {
    let base = api_base
        .filter(|base| !base.is_empty())
        .unwrap_or(DEFAULT_API_BASE);
    format!("{base}/image/v1/enhance")
}

// TopazImageVariationConfig.map_openai_params copies response_format and splits size without parsing its parts
pub fn map_openai_params(
    non_default_params: &ImageParams,
    mut optional_params: ImageParams,
) -> Result<ImageParams, TopazError> {
    for (name, value) in non_default_params {
        match name.as_str() {
            "response_format" => {
                optional_params.insert("output_format".to_owned(), value.clone());
            }
            "size" => {
                let Json::Text(size) = value else {
                    return Err(TopazError::SizeFormat);
                };
                let (width, height) = split_size(size)?;
                optional_params.insert("output_width".to_owned(), Json::Text(width.to_owned()));
                optional_params.insert("output_height".to_owned(), Json::Text(height.to_owned()));
            }
            _ => {}
        }
    }
    Ok(optional_params)
}

// the source splits on x and asserts exactly two parts, so "x" splits into two empty ones and "1024" does not split
fn split_size(size: &str) -> Result<(&str, &str), TopazError> {
    let mut parts = size.split('x');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(width), Some(height), None) => Ok((width, height)),
        _ => Err(TopazError::SizeFormat),
    }
}
