use std::ops::{Add as _, Mul as _};

use jiff::Timestamp;

use crate::email_date::{mktime_tz, parsedate_tz, python_int};

mod tests;

// LiteLLM reads these from INITIAL_RETRY_DELAY, MAX_RETRY_DELAY and JITTER, so the caller passes what it resolved.
#[derive(Clone, Copy, Debug)]
pub struct RetryTiming {
    pub initial_retry_delay: f64,
    pub max_retry_delay: f64,
    pub jitter: f64,
}

impl Default for RetryTiming {
    fn default() -> Self {
        Self {
            initial_retry_delay: 0.5_f64,
            max_retry_delay: 8.0_f64,
            jitter: 0.75_f64,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RetryAttempt {
    pub remaining_retries: i32,
    pub max_retries: i32,
    pub min_timeout: i32,
}

#[must_use]
pub const fn should_retry(status_code: u16) -> bool {
    match status_code {
        408 | 409 | 429 => true,
        code => code >= 500,
    }
}

// A whole number of seconds, or an email date turned into seconds from `now`; -1 when neither parses.
#[must_use]
pub fn retry_after_from_header(header: Option<&str>, now: Timestamp) -> i64 {
    let Some(text) = header else {
        return -1;
    };
    let trimmed = text.trim();
    if let Some(seconds) = python_int(trimmed) {
        return seconds;
    }
    let Some(parsed) = parsedate_tz(trimmed) else {
        return -1;
    };
    let Some(seconds) = mktime_tz(&parsed) else {
        return -1;
    };
    let Ok(date) = Timestamp::from_second(seconds) else {
        return -1;
    };
    date.duration_since(now).as_secs()
}

// `random_unit` is the value of random.random(), in 0 up to 1, so a caller or a test can pin the jitter.
#[must_use]
pub fn calculate_retry_after(
    timing: &RetryTiming,
    attempt: &RetryAttempt,
    retry_after_header: Option<&str>,
    now: Timestamp,
    random_unit: f64,
) -> f64 {
    let retry_after = retry_after_from_header(retry_after_header, now);
    let jitter = timing.jitter.mul(random_unit);
    if let Ok(seconds) = u8::try_from(retry_after)
        && (1..=60).contains(&seconds)
    {
        return f64::from(seconds).add(jitter);
    }
    let retries_made = attempt
        .max_retries
        .saturating_sub(attempt.remaining_retries);
    let backoff = timing
        .initial_retry_delay
        .mul(2.0_f64.powi(retries_made))
        .max(f64::from(attempt.min_timeout))
        .min(timing.max_retry_delay);
    backoff.add(jitter)
}
