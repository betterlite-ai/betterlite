use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneralSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneralSettings {
    #[serde(default, rename = "allow_requests_on_db_unavailable")]
    allow_requests: Entry<RawValue>,
}

// a key present with null differs from an absent key everywhere else, though both resolve to the setting's default here
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum Entry<T> {
    #[default]
    #[serde(skip_deserializing)]
    Absent,
    Null(()),
    Value(T),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawValue {
    Boolean(bool),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub allow_requests: AllowRequestsOnOutage,
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let allow_requests = match self.general_settings.allow_requests {
            Entry::Absent | Entry::Null(()) => AllowRequestsOnOutage::Boolean(false),
            Entry::Value(RawValue::Boolean(allow)) => AllowRequestsOnOutage::Boolean(allow),
            Entry::Value(RawValue::Text(text)) => AllowRequestsOnOutage::Text(resolve_string(
                text,
                "general_settings.allow_requests_on_db_unavailable",
                lookup,
            )?),
            Entry::Value(RawValue::Other(_)) => AllowRequestsOnOutage::Unsupported,
        };
        Ok(Settings { allow_requests })
    }
}
