use std::str::FromStr;
use std::time::Duration;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

mod tests;

const MAX_SERIES_PATH: &str = "litellm_settings.prometheus_end_user_metrics_max_series_per_metric";
const TTL_PATH: &str = "litellm_settings.prometheus_end_user_metrics_ttl_seconds";
const CLEANUP_PATH: &str = "litellm_settings.prometheus_end_user_metrics_cleanup_interval_seconds";

// litellm_settings.prometheus_end_user_metrics_max_series_per_metric = 10000 when the key is absent
const DEFAULT_MAX_SERIES: i64 = 10000;
// litellm_settings.prometheus_end_user_metrics_ttl_seconds = 3600 when the key is absent
const DEFAULT_TTL: Duration = Duration::from_secs(3600);
// litellm_settings.prometheus_end_user_metrics_cleanup_interval_seconds = 60 when the key is absent
const DEFAULT_CLEANUP: Duration = Duration::from_secs(60);

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    #[serde(default, rename = "prometheus_end_user_metrics_max_series_per_metric")]
    max_series: RawLimit,
    #[serde(default, rename = "prometheus_end_user_metrics_ttl_seconds")]
    ttl_seconds: RawLimit,
    #[serde(
        default,
        rename = "prometheus_end_user_metrics_cleanup_interval_seconds"
    )]
    cleanup_interval_seconds: RawLimit,
}

// Missing comes from the field default, so an absent key differs from an explicit null
#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum RawLimit {
    Number(RawNumber),
    Null(()),
    Other(IgnoredAny),
    #[default]
    Missing,
}

// max_series and ttl are None when their key is null; cleanup_interval is None for null, zero and negative, which all run the TTL pass on every call
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub max_series: Option<i64>,
    pub ttl: Option<Duration>,
    pub cleanup_interval: Option<Duration>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_series: Some(DEFAULT_MAX_SERIES),
            ttl: Some(DEFAULT_TTL),
            cleanup_interval: Some(DEFAULT_CLEANUP),
        }
    }
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = self.litellm_settings;
        let max_series = resolve_limit(
            raw.max_series,
            MAX_SERIES_PATH,
            lookup,
            DEFAULT_MAX_SERIES,
            parse_max_series,
        )?;
        let ttl = resolve_limit(
            raw.ttl_seconds,
            TTL_PATH,
            lookup,
            DEFAULT_TTL.as_secs_f64(),
            parse_float,
        )?
        .map(|seconds| seconds_duration(seconds, TTL_PATH))
        .transpose()?;
        let cleanup_interval = resolve_limit(
            raw.cleanup_interval_seconds,
            CLEANUP_PATH,
            lookup,
            DEFAULT_CLEANUP.as_secs_f64(),
            parse_float,
        )?
        .map(|seconds| cleanup_duration(seconds, CLEANUP_PATH))
        .transpose()?
        .flatten();
        Ok(Settings {
            max_series,
            ttl,
            cleanup_interval,
        })
    }
}

fn resolve_limit<T, L, F>(
    limit: RawLimit,
    path: &str,
    lookup: &L,
    default: T,
    parser: F,
) -> Result<Option<T>, ConfigError>
where
    T: FromStr,
    L: EnvironmentLookup,
    F: Fn(&str, &str) -> Result<T, ConfigError>,
{
    match limit {
        RawLimit::Missing => Ok(Some(default)),
        RawLimit::Null(()) => Ok(None),
        RawLimit::Other(_ignored) => Err(ConfigError::InvalidScalar {
            path: path.to_owned(),
            value: false,
        }),
        RawLimit::Number(number) => {
            // an environment reference whose value is empty counts as unset, so the key keeps its default
            resolve_number(Some(number), path, lookup, parser)?
                .map_or_else(|| Ok(Some(default)), |value| Ok(Some(value)))
        }
    }
}

fn parse_max_series(text: &str, path: &str) -> Result<i64, ConfigError> {
    text.parse::<i64>()
        .map_err(|source| ConfigError::InvalidInteger {
            path: path.to_owned(),
            source,
        })
}

fn seconds_duration(seconds: f64, path: &str) -> Result<Duration, ConfigError> {
    Duration::try_from_secs_f64(seconds).map_err(|_source| ConfigError::InvalidScalar {
        path: path.to_owned(),
        value: false,
    })
}

// a zero or negative interval cleans on every track_series call, which None already means, matching LiteLLM's _should_run_ttl_cleanup
fn cleanup_duration(seconds: f64, path: &str) -> Result<Option<Duration>, ConfigError> {
    if seconds <= 0.0_f64 {
        return Ok(None);
    }
    seconds_duration(seconds, path).map(Some)
}
