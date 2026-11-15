use crate::openai_base_config::Object;

use super::error::ModelScopeImageError;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 4] = ["n", "response_format", "size", "user"];

// LiteLLM's generic image path raises UnsupportedParamsError for an option the provider does not list
pub fn map_openai_params(
    non_default_params: &Object,
    optional_params: &mut Object,
    model: &str,
    drop_params: bool,
) -> Result<(), ModelScopeImageError> {
    for (name, value) in non_default_params {
        if !SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            if drop_params {
                continue;
            }
            return Err(ModelScopeImageError::UnsupportedParam {
                name: name.clone(),
                model: model.to_owned(),
            });
        }
        optional_params.insert(name.clone(), value.clone());
    }
    Ok(())
}
