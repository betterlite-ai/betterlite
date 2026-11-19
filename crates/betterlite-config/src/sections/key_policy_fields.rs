use fancy_regex::Regex;
use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;
use crate::sections::Section;
use crate::sections::settings::JsonValue;

mod tests;

const ENV_PREFIX: &str = "os.environ/";

pub const ALIAS_PATTERN_PATH: &str = "litellm_settings.key_alias_pattern";
pub const VALIDATION_PATH: &str = "litellm_settings.enable_key_alias_format_validation";
pub const ALIAS_MAX_LENGTH: usize = 255;
pub const BUILTIN_ALIAS_PATTERN: &str = r"^[a-zA-Z0-9][a-zA-Z0-9_\-/\.@]{0,253}[a-zA-Z0-9]$";
pub const BUILTIN_ALIAS_MESSAGE: &str = "Invalid key_alias format. Must be 2-255 characters, start/end with alphanumeric, and only contain a-zA-Z0-9_-/.@.";
pub const CONFIGURED_ALIAS_MESSAGE: &str = "Invalid key_alias format. Must be at most 255 characters and match the configured key_alias_pattern: ";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default, rename = "litellm_settings")]
    pub settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawSettings {
    #[serde(default, rename = "enable_key_alias_format_validation")]
    pub format_validation: Option<JsonValue>,
    #[serde(default, rename = "key_alias_pattern")]
    pub alias_pattern: Option<JsonValue>,
}

#[derive(Debug, Clone)]
pub struct AliasPattern {
    source: String,
    regex: Regex,
}

#[derive(Debug, Clone)]
pub enum AliasRule<'settings> {
    Configured(&'settings AliasPattern),
    Builtin,
}

impl AliasRule<'_> {
    #[must_use]
    pub fn accepts(&self, alias: &str) -> bool {
        if alias.chars().count() > ALIAS_MAX_LENGTH {
            return false;
        }
        match self {
            // a pattern that runs out of backtracking budget matches nothing, so a hostile alias can never widen the rule
            Self::Configured(pattern) => pattern.regex.is_match(alias).unwrap_or_default(),
            Self::Builtin => accepts_builtin(alias),
        }
    }

    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Configured(pattern) => {
                let source = pattern.source.as_str();
                format!("{CONFIGURED_ALIAS_MESSAGE}{source}")
            }
            Self::Builtin => BUILTIN_ALIAS_MESSAGE.to_owned(),
        }
    }
}

// the built-in rule is a character set and two endpoint checks, so it needs no pattern compile
fn accepts_builtin(alias: &str) -> bool {
    let mut characters = alias.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let Some(last) = characters.next_back() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '/' | '.' | '@')
        })
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub alias_format_validation: bool,
    pub alias_pattern: Option<AliasPattern>,
}

impl Settings {
    // the configured pattern wins over the toggle: LiteLLM reads key_alias_pattern before enable_key_alias_format_validation
    #[must_use]
    pub const fn alias_rule(&self) -> Option<AliasRule<'_>> {
        match (&self.alias_pattern, self.alias_format_validation) {
            (Some(pattern), _enabled) => Some(AliasRule::Configured(pattern)),
            (None, true) => Some(AliasRule::Builtin),
            (None, false) => None,
        }
    }
}

// an absent or null pattern leaves the rule unset; a string must compile, and the whole match span is anchored
pub fn parse_key_alias_pattern(value: JsonValue) -> Result<Option<AliasPattern>, ConfigError> {
    let invalid = || ConfigError::InvalidScalar {
        path: ALIAS_PATTERN_PATH.to_owned(),
        value: false,
    };
    let source = match value {
        JsonValue::Null => return Ok(None),
        JsonValue::String(source) => source,
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => {
            return Err(invalid());
        }
    };
    match Regex::new(&format!(r"\A(?:{source})\z")) {
        Ok(regex) => Ok(Some(AliasPattern { source, regex })),
        Err(_invalid) => Err(invalid()),
    }
}

// ProxyConfig resolves os.environ/ strings before parse_key_alias_pattern sees the value
fn configured_pattern<L>(value: JsonValue, lookup: &L) -> Result<Option<AliasPattern>, ConfigError>
where
    L: EnvironmentLookup,
{
    match value {
        JsonValue::String(text) => {
            let resolved = resolve_string(text, ALIAS_PATTERN_PATH, lookup)?;
            parse_key_alias_pattern(JsonValue::String(resolved))
        }
        other @ JsonValue::Null
        | other @ JsonValue::Bool(_)
        | other @ JsonValue::Number(_)
        | other @ JsonValue::Array(_)
        | other @ JsonValue::Object(_) => parse_key_alias_pattern(other),
    }
}

fn format_validation<L>(value: Option<JsonValue>, lookup: &L) -> Result<bool, ConfigError>
where
    L: EnvironmentLookup,
{
    let invalid = || ConfigError::InvalidScalar {
        path: VALIDATION_PATH.to_owned(),
        value: false,
    };
    let Some(present) = value else {
        return Ok(false);
    };
    match present {
        JsonValue::Null => Ok(false),
        JsonValue::Bool(flag) => Ok(flag),
        JsonValue::String(text) => {
            let Some(variable) = text.strip_prefix(ENV_PREFIX) else {
                return Err(invalid());
            };
            let name = variable.to_owned();
            let resolved = resolve_string(text, VALIDATION_PATH, lookup)?;
            Ok(betterlite_env::flag(&name, &resolved)?.unwrap_or(false))
        }
        JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_) => Err(invalid()),
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = self.settings;
        Ok(Settings {
            alias_format_validation: format_validation(raw.format_validation, lookup)?,
            alias_pattern: raw
                .alias_pattern
                .map(|value| configured_pattern(value, lookup))
                .transpose()?
                .flatten(),
        })
    }
}
