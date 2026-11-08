use crate::custom_interface::CustomParams;

#[derive(Debug, Default)]
pub struct SecretManagerSettings {
    pub primary_secret_name: Option<String>,
    // LiteLLM hands the custom manager key_management_settings.model_dump(), which the caller builds
    pub optional_params: CustomParams,
}
