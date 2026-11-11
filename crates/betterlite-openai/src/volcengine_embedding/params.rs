use crate::openai_base_config::Object;

use super::error::VolcEngineEmbeddingError;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["encoding_format", "user", "extra_headers"];

const ENCODING_FORMATS: [&str; 2] = ["float", "base64"];

// an unsupported option and a bad encoding_format are both dropped only when drop_params is set
pub fn map_openai_params(
    non_default_params: &Object,
    optional_params: &mut Object,
    drop_params: bool,
) -> Result<(), VolcEngineEmbeddingError> {
    for (name, value) in non_default_params {
        match name.as_str() {
            "encoding_format" => {
                let accepted = value.is_null()
                    || value
                        .as_str()
                        .is_some_and(|format| ENCODING_FORMATS.contains(&format));
                if !accepted && !drop_params {
                    return Err(VolcEngineEmbeddingError::UnsupportedEncodingFormat {
                        value: value.clone(),
                    });
                }
                if accepted {
                    optional_params.insert(name.clone(), value.clone());
                }
            }
            "user" | "extra_headers" => {
                optional_params.insert(name.clone(), value.clone());
            }
            _ if !drop_params => {
                return Err(VolcEngineEmbeddingError::UnsupportedParam { name: name.clone() });
            }
            _ => {}
        }
    }
    Ok(())
}
