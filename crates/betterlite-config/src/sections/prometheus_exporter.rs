use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::Section;

mod tests;

const LATENCY_BUCKETS_PATH: &str = "litellm_settings.prometheus_latency_buckets";

#[derive(Debug, Default, Deserialize)]
pub struct Raw {
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    #[serde(default)]
    prometheus_latency_buckets: Option<RawBuckets>,
    #[serde(default)]
    prometheus_emit_input_sequence_length_label: Option<RawToggle>, // ast-grep-ignore: no-long-names
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawBuckets {
    List(Vec<RawNumber>),
    Other(IgnoredAny),
}

// litellm.prometheus_emit_input_sequence_length_label is tested with `is True`, so "true" stays off
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawToggle {
    Boolean(bool),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub prometheus_latency_buckets: Option<Vec<f64>>,
    pub prometheus_emit_input_sequence_length_label: bool, // ast-grep-ignore: no-long-names
}

impl Section for Raw {
    type Settings = Settings;

    fn convert<L>(self, lookup: &L) -> Result<Self::Settings, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let raw = self.litellm_settings;
        let toggle = raw.prometheus_emit_input_sequence_length_label; // ast-grep-ignore: no-long-names
        let emit_input_length = matches!(toggle, Some(RawToggle::Boolean(true)));
        Ok(Settings {
            prometheus_latency_buckets: convert_buckets(raw.prometheus_latency_buckets, lookup)?,
            prometheus_emit_input_sequence_length_label: emit_input_length, // ast-grep-ignore: no-long-names
        })
    }
}

fn convert_buckets<L>(
    buckets: Option<RawBuckets>,
    lookup: &L,
) -> Result<Option<Vec<f64>>, ConfigError>
where
    L: EnvironmentLookup,
{
    let numbers = match buckets {
        None => return Ok(None),
        Some(RawBuckets::List(numbers)) => numbers,
        Some(RawBuckets::Other(_ignored)) => {
            return Err(ConfigError::InvalidScalar {
                path: LATENCY_BUCKETS_PATH.to_owned(),
                value: false,
            });
        }
    };
    let mut seconds = Vec::with_capacity(numbers.len());
    for number in numbers {
        let resolved = resolve_number(Some(number), LATENCY_BUCKETS_PATH, lookup, parse_float)?;
        seconds.push(resolved.ok_or_else(|| ConfigError::InvalidScalar {
            path: LATENCY_BUCKETS_PATH.to_owned(),
            value: false,
        })?);
    }
    Ok(Some(seconds))
}
