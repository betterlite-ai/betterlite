mod tests;

// the cancel slice reads the same litellm_settings and environment keys as the SDK resource settings
pub type Raw = crate::sections::resource_sdk_settings::Raw;
pub type Settings = crate::sections::resource_sdk_settings::Settings;
