use betterlite_models::decisions::Json;
use betterlite_secret_managers::secret_lookup::text_to_bool;
use serde::Deserialize;
use tracing::warn;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;

mod tests;

// login_throttle.py's defaults and setting names, shared with the throttle itself
pub const DEFAULT_SOURCE_LIMIT: i64 = 10;
pub const DEFAULT_WINDOW_SECONDS: i64 = 60;
pub const DEFAULT_BLOCK_SECONDS: i64 = 300;
pub const SOURCE_LIMIT_KEY: &str = "max_failed_login_attempts_per_source";
pub const SOURCE_LIMIT_OVERRIDES_KEY: &str = "max_failed_login_attempts_per_source_overrides";
pub const WINDOW_KEY: &str = "failed_login_window_seconds";
pub const BLOCK_KEY: &str = "failed_login_block_seconds";
pub const TRUSTED_PROXY_RANGES_KEY: &str = "trusted_proxy_ranges";
pub const DISABLE_ENV: &str = "LITELLM_DISABLE_LOGIN_RATE_LIMIT";

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    general_settings: RawGeneral,
}

// every key stays raw: int()'s string rules, the override values and the topology entries are decoded here
#[derive(Clone, Debug, Default, Deserialize)]
struct RawGeneral {
    #[serde(default, rename = "max_failed_login_attempts_per_source")]
    source_limit: Option<Json>,
    #[serde(default, rename = "max_failed_login_attempts_per_source_overrides")]
    source_limit_overrides: Option<Json>,
    #[serde(default)]
    failed_login_window_seconds: Option<Json>,
    #[serde(default)]
    failed_login_block_seconds: Option<Json>,
    #[serde(default)]
    trusted_proxy_ranges: Option<Json>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub source_limit: i64,
    pub source_limit_overrides: SourceOverrides,
    pub window_seconds: i64,
    pub block_seconds: i64,
    pub proxy_topology: ProxyTopology,
    pub disabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            source_limit: DEFAULT_SOURCE_LIMIT,
            source_limit_overrides: SourceOverrides::Absent,
            window_seconds: DEFAULT_WINDOW_SECONDS,
            block_seconds: DEFAULT_BLOCK_SECONDS,
            proxy_topology: ProxyTopology::Unknown,
            disabled: false,
        }
    }
}

// None and a null value both leave the source allowance at its default
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SourceOverrides {
    #[default]
    Absent,
    Invalid,
    Mapping(Vec<SourceOverride>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceOverride {
    pub range: String,
    pub limit: OverrideLimit,
}

// the values are decoded with _override_limit's rules; an invalid value waits for a matching address to warn
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverrideLimit {
    Exempt,
    Limit(i64),
    Invalid,
}

// an absent key, a null, a wrong outer shape and entries are all distinguishable before validation
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProxyTopology {
    #[default]
    Unknown,
    Entries(Vec<String>),
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let general = self.general_settings;
        // the order matches one from_request pass: topology, source allowance, window, then block
        let proxy_topology = topology(general.trusted_proxy_ranges, lookup);
        let source_limit = positive_int(
            general.source_limit,
            SOURCE_LIMIT_KEY,
            DEFAULT_SOURCE_LIMIT,
            lookup,
        );
        let source_limit_overrides =
            overrides(general.source_limit_overrides, source_limit, lookup);
        let window_seconds = positive_int(
            general.failed_login_window_seconds,
            WINDOW_KEY,
            DEFAULT_WINDOW_SECONDS,
            lookup,
        );
        let block_seconds = positive_int(
            general.failed_login_block_seconds,
            BLOCK_KEY,
            DEFAULT_BLOCK_SECONDS,
            lookup,
        );
        Ok(Settings {
            source_limit,
            source_limit_overrides,
            window_seconds,
            block_seconds,
            proxy_topology,
            // get_secret_bool reads only an exact trimmed true as disabled
            disabled: lookup
                .lookup(DISABLE_ENV)
                .and_then(|text| text_to_bool(&text))
                == Some(true),
        })
    }
}

// the proxy resolves os.environ/ references across the loaded config before any request reads a setting
fn resolved_environment(value: Json, lookup: &dyn EnvironmentLookup) -> Json {
    match value {
        Json::Text(text) => {
            let reference = text.strip_prefix("os.environ/").map(str::to_owned);
            reference.map_or_else(
                || Json::Text(text),
                |name| lookup.lookup(&name).map_or(Json::Null, Json::Text),
            )
        }
        other
        @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => other,
    }
}

// absent and null both leave the setting at its default; an os.environ/ reference resolves first
fn resolved<L>(raw: Option<Json>, lookup: &L) -> Option<Json>
where
    L: EnvironmentLookup,
{
    match resolved_environment(raw?, lookup) {
        Json::Null => None,
        value @ (Json::Bool(_)
        | Json::Number(_)
        | Json::Text(_)
        | Json::List(_)
        | Json::Object(_)) => Some(value),
    }
}

// _configured_range_entries: a string splits on commas, an iterable stringifies its entries, anything else is unknown
fn topology<L>(raw: Option<Json>, lookup: &L) -> ProxyTopology
where
    L: EnvironmentLookup,
{
    let Some(value) = resolved(raw, lookup) else {
        return ProxyTopology::Unknown;
    };
    if let Json::Text(text) = &value {
        return ProxyTopology::Entries(
            text.split(',').map(|part| part.trim().to_owned()).collect(),
        );
    }
    if let Json::List(items) = &value {
        return ProxyTopology::Entries(
            items
                .iter()
                .map(|item| text_of(item).trim().to_owned())
                .collect(),
        );
    }
    warn!(
        "Invalid {TRUSTED_PROXY_RANGES_KEY} value: expected a list of address ranges, got a scalar or mapping"
    );
    ProxyTopology::Unknown
}

// _positive_int: int(str(value)) at least 1, otherwise the default with a warning
fn positive_int<L>(raw: Option<Json>, key: &str, default: i64, lookup: &L) -> i64
where
    L: EnvironmentLookup,
{
    let Some(raw_value) = resolved(raw, lookup) else {
        return default;
    };
    let text = text_of(&raw_value);
    match int_of(&text) {
        Some(value) if value >= 1 => value,
        Some(value) => {
            warn!("Invalid {key} value {value} (must be >= 1); using {default}");
            default
        }
        None => {
            warn!("Invalid {key} value {text}; using {default}");
            default
        }
    }
}

// _source_limit keeps the mapping order; the throttle only evaluates a value whose range matches
fn overrides<L>(raw: Option<Json>, default: i64, lookup: &L) -> SourceOverrides
where
    L: EnvironmentLookup,
{
    let Some(value) = resolved(raw, lookup) else {
        return SourceOverrides::Absent;
    };
    let Json::Object(entries) = value else {
        warn!(
            "Invalid {SOURCE_LIMIT_OVERRIDES_KEY} value; expected a mapping of address or range to limit"
        );
        return SourceOverrides::Invalid;
    };
    SourceOverrides::Mapping(
        entries
            .into_iter()
            .map(|(range, raw_value)| SourceOverride {
                range,
                limit: override_limit(&resolved_environment(raw_value, lookup), default),
            })
            .collect(),
    )
}

// _override_limit: a null value silently keeps the source default; only a stripped 0 exempts, otherwise the value must be a positive integer
fn override_limit(raw: &Json, default: i64) -> OverrideLimit {
    if *raw == Json::Null {
        return OverrideLimit::Limit(default);
    }
    let text = text_of(raw);
    if text.trim() == "0" {
        return OverrideLimit::Exempt;
    }
    match int_of(&text) {
        Some(value) if value >= 1 => OverrideLimit::Limit(value),
        Some(_) | None => OverrideLimit::Invalid,
    }
}

// str(value) as int() and _override_limit read it; the container repr only has to stay unparsable
fn text_of(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number.to_string(),
        Json::Text(text) => text.clone(),
        Json::List(_) => "[...]".to_owned(),
        Json::Object(_) => "{...}".to_owned(),
    }
}

// int(text) for a str: surrounding whitespace, one sign and digits in single-underscore groups
fn int_of(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (negative, unsigned) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    let mut digits = String::with_capacity(unsigned.len());
    for group in unsigned.split('_') {
        if group.is_empty() {
            return None;
        }
        if !group.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        digits.push_str(group);
    }
    let magnitude = digits.parse::<i64>().ok()?;
    if negative {
        magnitude.checked_neg()
    } else {
        Some(magnitude)
    }
}
