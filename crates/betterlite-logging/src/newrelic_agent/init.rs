use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use serde::Deserialize;
use serde_with::{As, DefaultOnNull};

pub const LICENSE_KEY_VARIABLE: &str = "NEW_RELIC_LICENSE_KEY";
pub const APP_NAME_VARIABLE: &str = "NEW_RELIC_APP_NAME";

// litellm.newrelic_params holds NewRelicInitParams, whose only field is this flag
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NewRelicInitParams {
    #[serde(deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub turn_off_message_logging: bool,
}

#[derive(Clone, Debug, Default)]
pub struct NewRelicInit {
    pub params: NewRelicInitParams,
    pub turn_off_message_logging: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct NewRelicSettings {
    pub license_key: Option<SecretString>,
    pub app_name: Option<String>,
    pub turn_off_message_logging: bool,
}

impl NewRelicSettings {
    #[must_use]
    pub fn resolve(init: &NewRelicInit, environment: &dyn EnvironmentLookup) -> Self {
        Self {
            license_key: environment
                .lookup(LICENSE_KEY_VARIABLE)
                .filter(|value| !value.is_empty())
                .map(SecretString::from),
            app_name: environment
                .lookup(APP_NAME_VARIABLE)
                .filter(|value| !value.is_empty()),
            turn_off_message_logging: init
                .turn_off_message_logging
                .unwrap_or(init.params.turn_off_message_logging),
        }
    }
}
