mod tests;

use crate::sections::resource_sdk_settings;

// the cancel slice reads the same litellm_settings and environment keys as the SDK resource settings
pub type Raw = resource_sdk_settings::Raw;
pub type Settings = resource_sdk_settings::Settings;
