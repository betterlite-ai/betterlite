use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;

mod tests;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default, rename = "disable_key_generate_for_org_admin")]
    key_generation_disabled: Entry<RawFlag>,
}

// a key present with null differs from an absent key, though both resolve to the setting's default here
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
enum RawFlag {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    List(Vec<IgnoredAny>),
    Mapping(BTreeMap<String, IgnoredAny>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub key_generation_disabled: bool,
}

// LiteLLM keeps the org admin switch off unless Python's bool() of the runtime value is true
fn flag_value<L>(flag: RawFlag, path: &str, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(match flag {
        RawFlag::Boolean(value) => value,
        RawFlag::Integer(value) => value != 0,
        RawFlag::Float(value) => value != 0.0,
        RawFlag::Text(text) => {
            // LiteLLM resolves an unset os.environ reference to None, which bool() reads as false
            let unset = text
                .strip_prefix("os.environ/")
                .is_some_and(|variable| lookup.lookup(variable).is_none());
            !unset && !resolve_string(text, path, lookup)?.is_empty()
        }
        RawFlag::List(items) => !items.is_empty(),
        RawFlag::Mapping(entries) => !entries.is_empty(),
    })
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let path = "general_settings.disable_key_generate_for_org_admin";
        let disabled = match self.general_settings.key_generation_disabled {
            Entry::Absent | Entry::Null(()) => false,
            Entry::Value(flag) => flag_value(flag, path, lookup)?,
        };
        Ok(Settings {
            key_generation_disabled: disabled,
        })
    }
}
