use std::collections::BTreeMap;

use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

const TEST_EMAIL_ADDRESS: &str = "TEST_EMAIL_ADDRESS";

// env-only section: general_settings.alerting belongs to alert_settings, so the loader reports every key
#[derive(Default, serde::Deserialize)]
#[serde(transparent)]
pub struct Raw(pub BTreeMap<String, Box<IgnoredAny>>);

#[derive(Clone, Debug, Default)]
pub struct Settings {
    pub test_email_address: Option<String>,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let test_email_address = lookup
            .lookup(TEST_EMAIL_ADDRESS)
            .filter(|value| !value.is_empty());
        Ok(Settings { test_email_address })
    }
}
