use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;

use crate::s3_transport::option_value::{OptionValue, integer_from_text};

#[cfg(test)]
mod tests;

const LOG_PROMPTS_ONLY_VARIABLE: &str = "S3_LOG_PROMPTS_ONLY";
const PARTITION_GRANULARITY_VARIABLE: &str = "S3_PARTITION_GRANULARITY";
const MAX_ADAPTIVE_CONCURRENCY_VARIABLE: &str = "DEFAULT_S3_MAX_ADAPTIVE_CONCURRENCY";
const DEFAULT_MAX_ADAPTIVE_CONCURRENCY: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartitionGranularity {
    Day,
    Hour,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    MaxConcurrentUploads,
    MaxQueueSize,
    MaxAdaptiveConcurrency,
}

impl Bound {
    const fn setting(self) -> &'static str {
        match self {
            Self::MaxConcurrentUploads => "s3_max_concurrent_uploads",
            Self::MaxQueueSize => "s3_max_queue_size",
            Self::MaxAdaptiveConcurrency => "s3_max_adaptive_concurrency",
        }
    }

    const fn rejects_bool(self) -> bool {
        match self {
            Self::MaxConcurrentUploads => false,
            Self::MaxQueueSize | Self::MaxAdaptiveConcurrency => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    DropOnTerminalError,
    AdaptiveConcurrency,
    BatchFileUpload,
}

impl Toggle {
    const fn setting(self) -> &'static str {
        match self {
            Self::DropOnTerminalError => "s3_drop_on_terminal_error",
            Self::AdaptiveConcurrency => "s3_adaptive_concurrency",
            Self::BatchFileUpload => "s3_batch_file_upload",
        }
    }

    const fn fallback(self) -> bool {
        match self {
            Self::DropOnTerminalError => true,
            Self::AdaptiveConcurrency | Self::BatchFileUpload => false,
        }
    }
}

#[must_use]
pub fn resolve_log_prompts_only(
    configured: Option<&OptionValue>,
    environment: &dyn EnvironmentLookup,
) -> bool {
    let raw = configured.cloned().or_else(|| {
        environment
            .lookup(LOG_PROMPTS_ONLY_VARIABLE)
            .map(OptionValue::Text)
    });
    match raw {
        None => false,
        Some(OptionValue::Text(text)) if text.is_empty() => false,
        Some(value) => value.as_bool().unwrap_or_else(|| {
            tracing::warn!(
                ?value,
                "s3 logging: s3_log_prompts_only is not a boolean, logging prompts only"
            );
            true
        }),
    }
}

#[must_use]
pub fn resolve_partition_granularity(
    configured: Option<&OptionValue>,
    environment: &dyn EnvironmentLookup,
) -> PartitionGranularity {
    let raw = configured.cloned().or_else(|| {
        environment
            .lookup(PARTITION_GRANULARITY_VARIABLE)
            .map(OptionValue::Text)
    });
    match raw {
        Some(OptionValue::Text(text)) if text == "hour" => PartitionGranularity::Hour,
        Some(OptionValue::Text(text)) if text.is_empty() || text == "day" => {
            PartitionGranularity::Day
        }
        None => PartitionGranularity::Day,
        Some(value) => {
            tracing::warn!(
                ?value,
                "s3 logging: s3_partition_granularity is not one of day, hour, using day"
            );
            PartitionGranularity::Day
        }
    }
}

#[must_use]
pub fn resolve_bound(bound: Bound, configured: Option<&OptionValue>, fallback: usize) -> usize {
    let setting = bound.setting();
    let value = match configured {
        None => return fallback,
        Some(OptionValue::Text(text)) if text.is_empty() => return fallback,
        Some(OptionValue::Bool(flag)) if bound.rejects_bool() => {
            tracing::warn!(
                setting,
                flag,
                fallback,
                "s3 logging: option is a boolean, not an integer"
            );
            return fallback;
        }
        Some(value) => value,
    };
    let Some(number) = value.as_integer() else {
        tracing::warn!(
            setting,
            ?value,
            fallback,
            "s3 logging: option is not an integer"
        );
        return fallback;
    };
    match usize::try_from(number) {
        Ok(resolved) if resolved >= 1 => resolved,
        Ok(_) | Err(_) => {
            tracing::warn!(
                setting,
                number,
                fallback,
                "s3 logging: option must be at least 1"
            );
            fallback
        }
    }
}

#[must_use]
pub fn resolve_max_retry_age(
    configured: Option<&OptionValue>,
    fallback: Option<Duration>,
) -> Option<Duration> {
    let value = match configured {
        None => return None,
        Some(OptionValue::Text(text)) if text.is_empty() => return None,
        Some(OptionValue::Bool(flag)) => {
            tracing::warn!(
                flag,
                ?fallback,
                "s3 logging: s3_max_retry_age_seconds is a boolean, not an integer"
            );
            return fallback;
        }
        Some(value) => value,
    };
    let Some(seconds) = value.as_integer() else {
        tracing::warn!(
            ?value,
            ?fallback,
            "s3 logging: s3_max_retry_age_seconds is not an integer"
        );
        return fallback;
    };
    match u64::try_from(seconds) {
        Ok(0) => None,
        Ok(age) => Some(Duration::from_secs(age)),
        Err(_) => {
            tracing::warn!(
                seconds,
                ?fallback,
                "s3 logging: s3_max_retry_age_seconds must be at least 0"
            );
            fallback
        }
    }
}

#[must_use]
pub fn resolve_toggle(toggle: Toggle, configured: Option<&OptionValue>) -> bool {
    match configured {
        None => toggle.fallback(),
        Some(OptionValue::Text(text)) if text.is_empty() => toggle.fallback(),
        Some(value) => value.as_bool().unwrap_or_else(|| {
            let (setting, fallback) = (toggle.setting(), toggle.fallback());
            tracing::warn!(
                setting,
                ?value,
                fallback,
                "s3 logging: option is not a boolean"
            );
            fallback
        }),
    }
}

#[must_use]
pub fn default_max_adaptive_concurrency(environment: &dyn EnvironmentLookup) -> usize {
    let configured = environment.lookup(MAX_ADAPTIVE_CONCURRENCY_VARIABLE);
    // LiteLLM reads this with int(), which refuses the decimal zeros pydantic accepts
    let parsed = configured
        .as_deref()
        .filter(|text| !text.contains('.'))
        .and_then(integer_from_text);
    match parsed.map(usize::try_from) {
        None => DEFAULT_MAX_ADAPTIVE_CONCURRENCY,
        Some(Ok(ceiling)) => ceiling,
        Some(Err(_)) => {
            tracing::warn!(
                ?configured,
                "s3 logging: DEFAULT_S3_MAX_ADAPTIVE_CONCURRENCY is negative, using 200"
            );
            DEFAULT_MAX_ADAPTIVE_CONCURRENCY
        }
    }
}
