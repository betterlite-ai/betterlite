use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::types::RoutingStrategy;
use betterlite_openai::api::image_contracts::records::{
    ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::RecraftError;
use super::reply::{RecraftReplyError, decode_recraft_images};
use super::settings::RecraftImageSettings;
use super::types::RecraftImageGenerationRequest;

#[derive(Debug, Clone)]
pub struct RecraftImageGenerationConfig {
    pub settings: RecraftImageSettings,
}

// LiteLLM's get_recraft_image_generation_config answers the same configuration for every model
#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_recraft_image_generation_config(
    _model: &str,
    lookup: &dyn EnvironmentLookup,
) -> RecraftImageGenerationConfig {
    let (api_key, api_base) = RoutingStrategy::recraft_image_settings(lookup);
    RecraftImageGenerationConfig {
        settings: RecraftImageSettings { api_key, api_base },
    }
}

impl RecraftImageGenerationConfig {
    // LiteLLM's get_supported_openai_params answers these four names for every model
    pub const SUPPORTED_OPENAI_PARAMS: [&'static str; 4] =
        ["n", "response_format", "size", "style"];

    pub const ENDPOINT: &'static str = "v1/images/generations";

    #[must_use]
    pub fn get_complete_url(&self, api_base: Option<&str>) -> String {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(self.settings.api_base.as_str());
        format!("{}/{}", base.trim_end_matches('/'), Self::ENDPOINT)
    }

    pub fn validate_environment(
        &self,
        mut headers: ImageHeaders,
        api_key: Option<&SecretString>,
    ) -> Result<ImageHeaders, RecraftError> {
        let key = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(self.settings.api_key.as_ref())
            .ok_or(RecraftError::MissingApiKey)?;
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
        Ok(headers)
    }

    pub fn map_openai_params(
        non_default_params: &ImageParams,
        mut optional_params: ImageParams,
        model: &str,
        drop_params: bool,
    ) -> Result<ImageParams, RecraftError> {
        for (name, value) in non_default_params {
            if optional_params.contains_key(name) {
                continue;
            }
            if !Self::SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
                if drop_params {
                    continue;
                }
                return Err(RecraftError::UnsupportedParameter {
                    parameter: name.clone(),
                    model: model.to_owned(),
                });
            }
            optional_params.insert(name.clone(), value.clone());
        }
        Ok(optional_params)
    }

    // ast-grep-ignore: no-long-names
    pub fn transform_image_generation_request(
        model: &str,
        prompt: &str,
        optional_params: &ImageParams,
    ) -> Result<RecraftImageGenerationRequest, RecraftError> {
        for argument in ["prompt", "model"] {
            if optional_params.contains_key(argument) {
                return Err(RecraftError::DuplicateArgument { argument });
            }
        }
        Ok(RecraftImageGenerationRequest {
            prompt: prompt.to_owned(),
            model: model.to_owned(),
            options: optional_params.clone(),
        })
    }

    // ast-grep-ignore: no-long-names
    pub fn transform_image_generation_response(
        response: &ImageHttpResponse,
        model_response: &mut ImageResponse,
    ) -> Result<(), RecraftReplyError> {
        let images = decode_recraft_images(response)?;
        model_response
            .data
            .get_or_insert_with(Vec::new)
            .extend(images);
        Ok(())
    }
}
