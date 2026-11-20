use indexmap::IndexMap;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
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
    Mapping(IndexMap<String, IgnoredAny>),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub key_generation_disabled: bool,
}

// LiteLLM keeps the org admin switch off unless Python's bool() of the runtime value is true
fn flag_value<L>(flag: RawFlag, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    Ok(match flag {
        RawFlag::Boolean(value) => value,
        RawFlag::Integer(value) => value != 0,
        RawFlag::Float(value) => value != 0.0,
        // an os.environ/ reference reads through the shared parser, an unset or empty one as the false default
        RawFlag::Text(text) => match text.strip_prefix("os.environ/") {
            Some(variable) => {
                betterlite_env::flag(variable, &lookup.lookup(variable).unwrap_or_default())?
                    .unwrap_or(false)
            }
            None => !text.is_empty(),
        },
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
        // absent and null both take the setting's false default
        let Entry::Value(flag) = self.general_settings.key_generation_disabled else {
            return Ok(Settings::default());
        };
        Ok(Settings {
            key_generation_disabled: flag_value(flag, lookup)?,
        })
    }
}
