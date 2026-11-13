use super::error::LocalServerError;
use crate::openai_base_config::Object;

// LiteLLM lists response_format twice and the list is part of its error text
pub const SUPPORTED_PARAMS: [&str; 4] = ["n", "response_format", "size", "response_format"];

pub fn map_openai_params(
    model: &str,
    non_default_params: &Object,
    optional_params: &mut Object,
    drop_params: bool,
) -> Result<(), LocalServerError> {
    for (param, value) in non_default_params {
        if optional_params.contains_key(param) {
            continue;
        }
        if SUPPORTED_PARAMS.contains(&param.as_str()) {
            optional_params.insert(param.clone(), value.clone());
            continue;
        }
        if !drop_params {
            return Err(LocalServerError::UnsupportedParam {
                param: param.clone(),
                model: model.to_owned(),
                supported: SUPPORTED_PARAMS.to_vec(),
            });
        }
    }
    Ok(())
}
