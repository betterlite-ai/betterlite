pub mod error;
pub mod reply;
pub mod types;

mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::types::Config;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use self::error::StabilityGenerationError;
// ast-grep-ignore: no-long-names
use self::types::StabilityImageGenerationRequest as GenerationRequest;
use self::types::{DEFAULT_OUTPUT_FORMAT, aspect_ratio, model_endpoint};

#[derive(Debug, Clone)]
pub struct StabilityGenerationSettings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl StabilityGenerationSettings {
    #[must_use]
    pub fn from_environment(lookup: &dyn EnvironmentLookup) -> Self {
        let (api_key, api_base) = Config::stability_generation_settings(lookup);
        Self { api_key, api_base }
    }
}

#[derive(Debug, Clone)]
pub struct StabilityImageGenerationConfig {
    pub settings: StabilityGenerationSettings,
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_stability_image_generation_config(
    _model: &str,
    lookup: &dyn EnvironmentLookup,
) -> StabilityImageGenerationConfig {
    StabilityImageGenerationConfig {
        settings: StabilityGenerationSettings::from_environment(lookup),
    }
}

impl StabilityImageGenerationConfig {
    // LiteLLM's get_supported_openai_params answers the same three names for every model
    pub const SUPPORTED_OPENAI_PARAMS: [&'static str; 3] = ["n", "size", "response_format"];

    // LiteLLM's use_multipart_form_data is true for Stability AI and false for a JSON provider
    pub const MULTIPART_FORM_DATA: bool = true;

    #[must_use]
    pub fn get_complete_url(&self, api_base: Option<&str>, model: &str) -> String {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(self.settings.api_base.as_str());
        format!("{}{}", base.trim_end_matches('/'), model_endpoint(model))
    }

    pub fn validate_environment(
        &self,
        headers: ImageHeaders,
        api_key: Option<&SecretString>,
    ) -> Result<ImageHeaders, StabilityGenerationError> {
        let key = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(self.settings.api_key.as_ref())
            .ok_or(StabilityGenerationError::MissingApiKey)?;
        let mut headers = headers;
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
        headers.insert("Accept".to_owned(), "application/json".to_owned());
        Ok(headers)
    }

    pub fn map_openai_params(
        non_default_params: &ImageParams,
        mut optional_params: ImageParams,
        model: &str,
        drop_params: bool,
    ) -> Result<ImageParams, StabilityGenerationError> {
        for (name, value) in non_default_params {
            if optional_params.contains_key(name) {
                continue;
            }
            match name.as_str() {
                "size" => match aspect_ratio(value) {
                    Some(ratio) => {
                        optional_params
                            .insert("aspect_ratio".to_owned(), Json::Text(ratio.to_owned()));
                    }
                    None => {
                        optional_params.insert(name.clone(), value.clone());
                    }
                },
                "n" => {
                    optional_params.insert("_n".to_owned(), value.clone());
                }
                "response_format" => {
                    optional_params.insert("_response_format".to_owned(), value.clone());
                }
                _ if drop_params => {}
                _ => {
                    return Err(StabilityGenerationError::UnsupportedParameter {
                        parameter: name.clone(),
                        model: model.to_owned(),
                    });
                }
            }
        }
        Ok(optional_params)
    }

    // ast-grep-ignore: no-long-names
    pub fn transform_image_generation_request(
        prompt: &str,
        optional_params: &ImageParams,
    ) -> Result<GenerationRequest, StabilityGenerationError> {
        let output_format = Some(
            field_value(
                "output_format",
                optional_params.get("output_format"),
                read_text,
            )?
            .unwrap_or_else(|| Some(DEFAULT_OUTPUT_FORMAT.to_owned())),
        );
        Ok(GenerationRequest {
            prompt: prompt.to_owned(),
            negative_prompt: field_value(
                "negative_prompt",
                optional_params.get("negative_prompt"),
                read_text,
            )?,
            aspect_ratio: field_value(
                "aspect_ratio",
                optional_params.get("aspect_ratio"),
                read_text,
            )?,
            seed: field_value("seed", optional_params.get("seed"), read_integer)?,
            output_format,
            model: field_value("model", optional_params.get("model"), read_text)?,
            mode: field_value("mode", optional_params.get("mode"), read_text)?,
            image: None,
            strength: field_value("strength", optional_params.get("strength"), read_number)?,
            style_preset: field_value(
                "style_preset",
                optional_params.get("style_preset"),
                read_text,
            )?,
        })
    }
}

// absent stays absent, an explicit null is kept, and any other shape names its field
fn field_value<T>(
    field: &'static str,
    value: Option<&Json>,
    read: fn(&Json) -> Option<T>,
) -> Result<Option<Option<T>>, StabilityGenerationError> {
    match value {
        None => Ok(None),
        Some(Json::Null) => Ok(Some(None)),
        Some(json) => read(json)
            .map(|found| Some(Some(found)))
            .ok_or(StabilityGenerationError::RequestField { field }),
    }
}

fn read_text(value: &Json) -> Option<String> {
    match value {
        Json::Text(text) => Some(text.clone()),
        _ => None,
    }
}

fn read_integer(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        _ => None,
    }
}

fn read_number(value: &Json) -> Option<Number> {
    match value {
        Json::Number(number) => Some(number.clone()),
        _ => None,
    }
}
