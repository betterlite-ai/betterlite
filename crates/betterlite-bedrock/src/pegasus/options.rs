use crate::image_models::params::Params;
use crate::pegasus::error::PegasusError;
use crate::pegasus::response_format::normalize_response_format;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 4] = [
    "max_tokens",
    "max_completion_tokens",
    "temperature",
    "response_format",
];

pub fn map_openai_params(non_default: &Params, optional: Params) -> Result<Params, PegasusError> {
    let mut mapped = optional;
    for (param, value) in non_default {
        match param.as_str() {
            "max_tokens" | "max_completion_tokens" => {
                drop(mapped.insert("maxOutputTokens".to_owned(), value.clone()));
            }
            "temperature" => {
                drop(mapped.insert("temperature".to_owned(), value.clone()));
            }
            "response_format" => {
                let normalized = normalize_response_format(value)?;
                drop(mapped.insert("responseFormat".to_owned(), normalized));
            }
            _ => {}
        }
    }
    Ok(mapped)
}
