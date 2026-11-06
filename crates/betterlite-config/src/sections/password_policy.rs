use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{resolve_number, resolve_string};
use crate::sections::Section;

mod tests;

const DEFAULT_MIN_LENGTH: u32 = 12;
const MIN_LENGTH_FLOOR: u32 = 8;

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(rename = "password_policy_min_length")]
    min_length: Option<RawNumber>,
    #[serde(rename = "password_policy_require_uppercase")]
    uppercase: Option<Flag>,
    #[serde(rename = "password_policy_require_lowercase")]
    lowercase: Option<Flag>,
    #[serde(rename = "password_policy_require_numbers")]
    numbers: Option<Flag>,
    #[serde(rename = "password_policy_require_special_characters")]
    special_characters: Option<Flag>,
    #[serde(rename = "password_policy_check_breached_passwords")]
    check_breached_passwords: Option<Flag>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Flag {
    Literal(bool),
    Text(String),
    Integer(i64),
    Float(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterClass {
    Uppercase,
    Lowercase,
    Number,
    Special,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub min_length: u32,
    pub required: Vec<CharacterClass>,
    pub check_breached_passwords: bool,
}

// The shared Sections snapshot requires Default; the policy defaults are the pinned LiteLLM ones.
impl Default for Settings {
    fn default() -> Self {
        Self {
            min_length: DEFAULT_MIN_LENGTH,
            required: vec![
                CharacterClass::Uppercase,
                CharacterClass::Lowercase,
                CharacterClass::Number,
                CharacterClass::Special,
            ],
            check_breached_passwords: true,
        }
    }
}

fn parse_length(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

fn resolve_flag(
    value: Option<&Flag>,
    field: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<bool, ConfigError> {
    let path = format!("general_settings.{field}");
    let invalid = |truthy: bool| ConfigError::InvalidScalar {
        path: path.clone(),
        value: truthy,
    };
    match value {
        None => Ok(true),
        Some(Flag::Literal(flag)) => Ok(*flag),
        Some(Flag::Integer(number)) => Err(invalid(*number != 0)),
        Some(Flag::Float(number)) => Err(invalid(*number != 0.0)),
        Some(Flag::Text(reference)) => {
            let Some(variable) = reference.strip_prefix("os.environ/") else {
                return Err(invalid(false));
            };
            let text = resolve_string(reference.clone(), &path, lookup)?;
            Ok(betterlite_env::flag(variable, &text)?.unwrap_or(true))
        }
    }
}

fn required_classes(
    general: &RawGeneral,
    lookup: &dyn EnvironmentLookup,
) -> Result<Vec<CharacterClass>, ConfigError> {
    let classes = [
        (
            CharacterClass::Uppercase,
            general.uppercase.as_ref(),
            "password_policy_require_uppercase",
        ),
        (
            CharacterClass::Lowercase,
            general.lowercase.as_ref(),
            "password_policy_require_lowercase",
        ),
        (
            CharacterClass::Number,
            general.numbers.as_ref(),
            "password_policy_require_numbers",
        ),
        (
            CharacterClass::Special,
            general.special_characters.as_ref(),
            "password_policy_require_special_characters",
        ),
    ];
    let mut required = Vec::new();
    for (class, flag, field) in classes {
        if resolve_flag(flag, field, lookup)? {
            required.push(class);
        }
    }
    Ok(required)
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        let required = required_classes(&general, lookup)?;
        let length = resolve_number(
            general.min_length,
            "general_settings.password_policy_min_length",
            lookup,
            parse_length,
        )?;
        let min_length = length.map_or(DEFAULT_MIN_LENGTH, |configured| {
            u32::try_from(configured.max(i64::from(MIN_LENGTH_FLOOR))).unwrap_or(u32::MAX)
        });
        Ok(Settings {
            min_length,
            required,
            check_breached_passwords: resolve_flag(
                general.check_breached_passwords.as_ref(),
                "password_policy_check_breached_passwords",
                lookup,
            )?,
        })
    }
}
