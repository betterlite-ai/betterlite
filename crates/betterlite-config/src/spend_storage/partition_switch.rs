use betterlite_db::spend_retention::batches::settings::SettingValue;

use crate::loader::ConfigError;

pub(super) const USE_SPEND_LOGS_PARTITIONING_PATH: &str =
    "general_settings.use_spend_logs_partitioning";

// litellm declares the switch as bool | None, so only a boolean or a null loads and everything else names the field
pub(super) fn load_switch(
    configured: Option<SettingValue>,
) -> Result<Option<SettingValue>, ConfigError> {
    match configured {
        None => Ok(None),
        Some(flag @ SettingValue::Flag(_)) => Ok(Some(flag)),
        Some(SettingValue::Number(_) | SettingValue::Text(_) | SettingValue::Nested(_)) => {
            Err(ConfigError::InvalidScalar {
                path: USE_SPEND_LOGS_PARTITIONING_PATH.to_owned(),
                value: false,
            })
        }
    }
}
