use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

const DEFAULT_WINDOW_SECONDS: u32 = 300;

#[derive(Debug, thiserror::Error)]
enum LaxCoercionError {
    #[error("not a recognized boolean")]
    Bool,
    #[error("not a non-negative integer")]
    WindowSeconds,
    #[error("not an integer")]
    Offset,
}

// Pydantic's lax mode accepts these case-insensitive strings for a bool field, plus literal ints 0/1.
const LAX_TRUE_STRINGS: [&str; 6] = ["true", "1", "yes", "y", "on", "t"];
const LAX_FALSE_STRINGS: [&str; 6] = ["false", "0", "no", "n", "off", "f"];

fn lax_bool_from_str(text: &str) -> Option<bool> {
    if LAX_TRUE_STRINGS
        .iter()
        .any(|candidate| text.eq_ignore_ascii_case(candidate))
    {
        Some(true)
    } else if LAX_FALSE_STRINGS
        .iter()
        .any(|candidate| text.eq_ignore_ascii_case(candidate))
    {
        Some(false)
    } else {
        None
    }
}

// Pydantic's lax mode parses a string int field the way Python's int() literal does.
fn lax_i64_from_str(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (is_negative, digits) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || digits.contains("__")
        || !digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_')
    {
        return None;
    }
    let cleaned: String = digits
        .chars()
        .filter(|&character| character != '_')
        .collect();
    let magnitude: i64 = cleaned.parse().ok()?;
    Some(if is_negative {
        magnitude.checked_neg()?
    } else {
        magnitude
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LaxBoolWire {
    Bool(bool),
    Int(i64),
    Str(String),
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "LaxBoolWire")]
struct LaxBool(bool);

impl TryFrom<LaxBoolWire> for LaxBool {
    type Error = LaxCoercionError;

    fn try_from(wire: LaxBoolWire) -> Result<Self, Self::Error> {
        match wire {
            LaxBoolWire::Bool(value) => Ok(Self(value)),
            LaxBoolWire::Int(0) => Ok(Self(false)),
            LaxBoolWire::Int(1) => Ok(Self(true)),
            LaxBoolWire::Int(_) => Err(LaxCoercionError::Bool),
            LaxBoolWire::Str(text) => lax_bool_from_str(&text)
                .map(Self)
                .ok_or(LaxCoercionError::Bool),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LaxIntWire {
    Int(i64),
    Str(String),
}

fn lax_i64_from_wire(wire: LaxIntWire) -> Option<i64> {
    match wire {
        LaxIntWire::Int(value) => Some(value),
        LaxIntWire::Str(text) => lax_i64_from_str(&text),
    }
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "LaxIntWire")]
struct LaxWindowSeconds(u32);

impl TryFrom<LaxIntWire> for LaxWindowSeconds {
    type Error = LaxCoercionError;

    fn try_from(wire: LaxIntWire) -> Result<Self, Self::Error> {
        let parsed = lax_i64_from_wire(wire).ok_or(LaxCoercionError::WindowSeconds)?;
        u32::try_from(parsed)
            .map(Self)
            .map_err(|_err| LaxCoercionError::WindowSeconds)
    }
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "LaxIntWire")]
struct LaxOffset(i64);

impl TryFrom<LaxIntWire> for LaxOffset {
    type Error = LaxCoercionError;

    fn try_from(wire: LaxIntWire) -> Result<Self, Self::Error> {
        lax_i64_from_wire(wire)
            .map(Self)
            .ok_or(LaxCoercionError::Offset)
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

#[derive(Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default)]
    scheduled_job_stagger: Option<RawStaggerNode>,
}

// one field in the wrong shape falls back to IgnoredAny (a storable serde_json::Value substitute) instead of invalidating the whole document
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawStaggerNode {
    Fields(StaggerFields),
    Other(IgnoredAny),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct StaggerFields {
    enabled: LaxBool,
    window_seconds: LaxWindowSeconds,
    identity: Option<String>,
    offsets: BTreeMap<String, LaxOffset>,
}

impl Default for StaggerFields {
    fn default() -> Self {
        Self {
            enabled: LaxBool(true),
            window_seconds: LaxWindowSeconds(DEFAULT_WINDOW_SECONDS),
            identity: None,
            offsets: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub window_seconds: u32,
    pub identity: Option<String>,
    pub offsets: BTreeMap<String, i64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from(StaggerFields::default())
    }
}

impl From<StaggerFields> for Settings {
    fn from(fields: StaggerFields) -> Self {
        Self {
            enabled: fields.enabled.0,
            window_seconds: fields.window_seconds.0,
            identity: fields.identity,
            offsets: fields
                .offsets
                .into_iter()
                .map(|(job_id, LaxOffset(offset))| (job_id, offset))
                .collect(),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, _lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        Ok(match self.general_settings.scheduled_job_stagger {
            None => Settings::default(),
            Some(RawStaggerNode::Fields(fields)) => Settings::from(fields),
            Some(RawStaggerNode::Other(_)) => {
                tracing::warn!(
                    "ignoring invalid general_settings.scheduled_job_stagger, falling back to defaults"
                );
                Settings::default()
            }
        })
    }
}

mod defaults;
mod invalid;
