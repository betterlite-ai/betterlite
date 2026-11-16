use betterlite_models::decisions::Json;

use super::error::DashscopeImageError;
use crate::api::image_contracts::records::ImageParams;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 2] = ["n", "size"];

// LiteLLM looks a size up in this table and rewrites every x of any other size string
const OPENAI_TO_DASHSCOPE_SIZE: [(&str, &str); 6] = [
    ("256x256", "256*256"),
    ("512x512", "512*512"),
    ("1024x1024", "1024*1024"),
    ("1792x1024", "1792*1024"),
    ("1024x1792", "1024*1792"),
    ("2048x2048", "2048*2048"),
];

// LiteLLM returns only the entries it mapped, not optional_params merged with them
pub fn map_openai_params(
    non_default_params: &ImageParams,
    optional_params: &ImageParams,
) -> Result<ImageParams, DashscopeImageError> {
    let mut mapped = ImageParams::new();
    for (name, value) in non_default_params {
        if optional_params.contains_key(name) {
            continue;
        }
        match name.as_str() {
            "n" => {
                mapped.insert(name.clone(), value.clone());
            }
            "size" => {
                mapped.insert(name.clone(), dashscope_size(value)?);
            }
            _ => {}
        }
    }
    Ok(mapped)
}

fn dashscope_size(value: &Json) -> Result<Json, DashscopeImageError> {
    let Json::Text(size) = value else {
        return Err(DashscopeImageError::ParamType { name: "size" });
    };
    let mapped = OPENAI_TO_DASHSCOPE_SIZE
        .iter()
        .find(|(openai, _)| *openai == size.as_str())
        .map_or_else(
            || size.replace('x', "*"),
            |(_, dashscope)| (*dashscope).to_owned(),
        );
    Ok(Json::Text(mapped))
}
