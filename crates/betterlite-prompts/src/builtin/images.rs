use super::{PromptError, bounds};
mod tests;

pub fn convert_to_ollama_image(value: &str) -> Result<String, PromptError> {
    if value.len() > bounds::OUTPUT_BYTES {
        return Err(PromptError::Limit);
    }
    if value.starts_with("http") {
        return Err(PromptError::RemoteImage);
    }
    let encoded = if let Some(image) = value.strip_prefix("data:image/") {
        image
            .split("data:image/")
            .next()
            .and_then(|segment| segment.split(";base64,").nth(1))
            .ok_or(PromptError::Image)?
    } else {
        value
    };
    Ok(encoded.to_owned())
}
