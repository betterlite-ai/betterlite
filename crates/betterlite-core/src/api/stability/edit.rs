pub mod error;
pub mod reply;
pub mod request;
pub mod types;

mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use secrecy::{ExposeSecret as _, SecretString};

use self::error::StabilityEditError;
use self::types::{DEFAULT_API_BASE, EditOptions, EditValue, model_endpoint};
use crate::api::stability::generation::types::aspect_ratio;

const API_KEY_VAR: &str = "STABILITY_API_KEY";
const API_BASE_VAR: &str = "STABILITY_API_BASE";

#[derive(Debug, Clone)]
pub struct StabilityEditSettings {
    pub api_key: Option<SecretString>,
    // STABILITY_API_BASE only when it is set: the default must not shadow litellm_params.api_base
    pub api_base: Option<String>,
}

impl StabilityEditSettings {
    #[must_use]
    pub fn from_environment(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(API_KEY_VAR).map(SecretString::from),
            api_base: present(API_BASE_VAR),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StabilityImageEditConfig {
    pub settings: StabilityEditSettings,
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn get_stability_image_edit_config(
    _model: &str,
    lookup: &dyn EnvironmentLookup,
) -> StabilityImageEditConfig {
    StabilityImageEditConfig {
        settings: StabilityEditSettings::from_environment(lookup),
    }
}

impl StabilityImageEditConfig {
    // LiteLLM's get_supported_openai_params answers the same four names for every edit model
    pub const SUPPORTED_OPENAI_PARAMS: [&'static str; 4] = ["n", "size", "response_format", "mask"];

    // LiteLLM's use_multipart_form_data is true for Stability AI and false for a JSON provider
    pub const MULTIPART_FORM_DATA: bool = true;

    // LiteLLM's get_complete_url: caller base, STABILITY_API_BASE, litellm_params.api_base, default
    #[must_use]
    pub fn get_complete_url(
        &self,
        api_base: Option<&str>,
        litellm_params: &ImageParams,
        model: &str,
    ) -> String {
        let litellm_base = litellm_params_api_base(litellm_params);
        let base = api_base
            .filter(|base| !base.is_empty())
            .or(self.settings.api_base.as_deref())
            .or(litellm_base)
            .unwrap_or(DEFAULT_API_BASE);
        format!("{}{}", base.trim_end_matches('/'), model_endpoint(model))
    }

    pub fn validate_environment(
        &self,
        mut headers: ImageHeaders,
        api_key: Option<&SecretString>,
    ) -> Result<ImageHeaders, StabilityEditError> {
        let key = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(self.settings.api_key.as_ref())
            .ok_or(StabilityEditError::MissingApiKey)?;
        headers.insert(
            "Authorization".to_owned(),
            format!("Bearer {}", key.expose_secret()),
        );
        headers.insert("Accept".to_owned(), "application/json".to_owned());
        Ok(headers)
    }

    // LiteLLM's map_openai_params: a copy, the size mapping, the two stashed names, then their removal
    pub fn map_openai_params(
        image_edit_optional_params: &EditOptions,
        model: &str,
        drop_params: bool,
    ) -> Result<EditOptions, StabilityEditError> {
        let mut mapped = image_edit_optional_params.clone();
        for (name, value) in image_edit_optional_params {
            if name == "size" {
                map_size(value, &mut mapped);
            } else if name == "n" || name == "response_format" {
                mapped.insert(format!("_{name}"), value.clone());
            } else if !Self::SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) && !drop_params {
                return Err(StabilityEditError::UnsupportedParameter {
                    parameter: name.clone(),
                    model: model.to_owned(),
                });
            }
        }
        for removed in ["size", "n", "response_format"] {
            mapped.shift_remove(removed);
        }
        Ok(mapped)
    }
}

fn map_size(value: &EditValue, mapped: &mut EditOptions) {
    if let EditValue::Json(size) = value
        && let Some(ratio) = aspect_ratio(size)
    {
        mapped.insert(
            "aspect_ratio".to_owned(),
            EditValue::Json(Json::Text(ratio.to_owned())),
        );
    }
}

fn litellm_params_api_base(params: &ImageParams) -> Option<&str> {
    let Some(Json::Text(base)) = params.get("api_base") else {
        return None;
    };
    (!base.is_empty()).then_some(base.as_str())
}
