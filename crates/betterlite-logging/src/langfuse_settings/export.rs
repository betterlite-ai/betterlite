use betterlite_config::sections::langfuse_settings::Settings;

use super::LangfuseSettingsError;

const PROMPT_CACHE_TTL_VARIABLE: &str = "LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS";
const DEFAULT_TIMEOUT_SECONDS: f64 = 20.0;
const DEFAULT_MAX_RETRIES: u32 = 3;
const MAX_RETRIES: u32 = 1_000;
const DEFAULT_FLUSH_AT: usize = 512;
const MAX_QUEUE_SIZE: usize = 100_000;
const DEFAULT_PROMPT_CACHE_TTL: f64 = 60.0;

#[must_use]
pub fn flush_interval(settings: &Settings, default: u64) -> u64 {
    let Some(raw) = settings
        .flush_interval
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return default;
    };
    let trimmed = raw.trim();
    let digits = !trimmed.is_empty() && trimmed.bytes().all(|byte| byte.is_ascii_digit());
    match digits.then(|| trimmed.parse::<u64>().unwrap_or(u64::MAX)) {
        Some(seconds) if seconds > 0 => seconds,
        Some(_) | None => {
            tracing::warn!(
                "LANGFUSE_FLUSH_INTERVAL='{raw}' is not a whole number of seconds above 0; flushing every {default} s"
            );
            default
        }
    }
}

#[must_use]
pub fn flush_at(settings: &Settings) -> usize {
    let Some(raw) = settings.flush_at.as_deref() else {
        return DEFAULT_FLUSH_AT;
    };
    // The empty string counts as all digits, and usize::MAX then fails the range check.
    let trimmed = raw.trim();
    let digits = trimmed.bytes().all(|byte| byte.is_ascii_digit());
    match digits.then(|| trimmed.parse::<usize>().unwrap_or(usize::MAX)) {
        Some(batch) if (1..=MAX_QUEUE_SIZE).contains(&batch) => batch,
        Some(_) | None => {
            tracing::warn!(
                "LANGFUSE_FLUSH_AT='{raw}' is not a whole number between 1 and {MAX_QUEUE_SIZE}; exporting batches of {DEFAULT_FLUSH_AT}"
            );
            DEFAULT_FLUSH_AT
        }
    }
}

#[must_use]
pub fn sample_rate(settings: &Settings) -> f64 {
    let Some(raw) = settings.sample_rate.as_deref() else {
        return 1.0;
    };
    match raw.trim().parse::<f64>() {
        Ok(rate) if (0.0..=1.0).contains(&rate) => rate,
        Ok(_) | Err(_) => {
            tracing::warn!(
                "LANGFUSE_SAMPLE_RATE='{raw}' is not a number between 0.0 and 1.0; ignoring it and exporting every trace"
            );
            1.0
        }
    }
}

pub fn timeout_seconds(settings: &Settings) -> Result<f64, LangfuseSettingsError> {
    let Some(raw) = settings.timeout.as_deref() else {
        return Ok(DEFAULT_TIMEOUT_SECONDS);
    };
    raw.trim()
        .parse::<f64>()
        .map_err(|source| LangfuseSettingsError::InvalidTimeout {
            raw: raw.to_owned(),
            source,
        })
}

#[must_use]
pub fn max_retries(settings: &Settings) -> u32 {
    let Some(raw) = settings.max_retries.as_deref() else {
        return DEFAULT_MAX_RETRIES;
    };
    let trimmed = raw.trim();
    let digits = !trimmed.is_empty() && trimmed.bytes().all(|byte| byte.is_ascii_digit());
    if !digits {
        tracing::warn!(
            "LANGFUSE_MAX_RETRIES='{raw}' is not a whole number; retrying {DEFAULT_MAX_RETRIES} times"
        );
        return DEFAULT_MAX_RETRIES;
    }
    // A digit run too long for u32 is above the ceiling too, so u32::MAX stands in for it.
    let requested = trimmed.parse::<u32>().unwrap_or(u32::MAX);
    if requested > MAX_RETRIES {
        tracing::warn!(
            "LANGFUSE_MAX_RETRIES={requested} is above the ceiling; retrying {MAX_RETRIES} times"
        );
    }
    requested.min(MAX_RETRIES)
}

pub fn check_prompt_cache_ttl(settings: &Settings) -> Result<(), LangfuseSettingsError> {
    match settings.prompt_cache_ttl.as_deref() {
        Some(raw) if whole_number_seconds(raw).is_none() => {
            Err(LangfuseSettingsError::UnusablePromptCacheTtl {
                raw: raw.to_owned(),
            })
        }
        Some(_) | None => Ok(()),
    }
}

#[must_use]
pub fn prompt_cache_ttl_seconds(settings: &Settings) -> f64 {
    let Some(raw) = settings.prompt_cache_ttl.as_deref() else {
        return DEFAULT_PROMPT_CACHE_TTL;
    };
    match whole_number_seconds(raw) {
        Some(seconds) if seconds >= 0.0 => seconds,
        Some(_) | None => {
            tracing::warn!(
                "{PROMPT_CACHE_TTL_VARIABLE}='{raw}' is not a whole number of seconds at or above 0; caching prompts for {DEFAULT_PROMPT_CACHE_TTL:.0} s"
            );
            DEFAULT_PROMPT_CACHE_TTL
        }
    }
}

fn whole_number_seconds(raw: &str) -> Option<f64> {
    let trimmed = raw.trim();
    trimmed.parse::<i64>().ok()?;
    trimmed.parse::<f64>().ok()
}
