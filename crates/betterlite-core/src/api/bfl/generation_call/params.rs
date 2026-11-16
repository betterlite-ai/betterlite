use betterlite_config::types::LiteLlmParams;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;

#[derive(Debug, thiserror::Error)]
pub enum ParamsError {
    #[error("litellm_params.{field} must be a string")]
    FieldType { field: &'static str },
}

// LiteLLM reads litellm_params either as a mapping or as GenericLiteLLMParams; both yield the api key and base
#[derive(Debug, Clone, Default)]
pub struct CallParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

impl CallParams {
    pub fn from_dict(params: &IndexMap<String, Json>) -> Result<Self, ParamsError> {
        Ok(Self {
            api_key: read_field(params, "api_key")?.map(SecretString::from),
            api_base: read_field(params, "api_base")?,
        })
    }

    #[must_use]
    pub fn from_typed(params: &LiteLlmParams) -> Self {
        Self {
            api_key: params.api_key.clone(),
            api_base: params.api_base.clone(),
        }
    }
}

fn read_field(
    params: &IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<String>, ParamsError> {
    match params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(_) => Err(ParamsError::FieldType { field }),
    }
}
