#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::path::PathBuf;

use rand::TryRng;
use rstest::rstest;

use crate::retry_policy::backoff::{
    BackoffSettings, DEFAULT_INITIAL_RETRY_DELAY, DEFAULT_JITTER, DEFAULT_MAX_RETRY_DELAY,
};
use crate::retry_policy::eligibility::Availability;
use crate::retry_policy::policy::RetryPolicy;
use crate::retry_policy::retry_after::parse_retry_after;

mod retry_policy_behaviors;
mod retry_policy_boundaries;

const CONFIG_VALID: &str =
    include_str!("../../../betterlite-config/fuzz/corpus/config_yaml/retry-policy-valid.yaml");
const CONFIG_INVALID: &str =
    include_str!("../../../betterlite-config/fuzz/corpus/config_yaml/retry-policy-invalid.yaml");

// a sample stands for exactly k / 2^53, Python's random() draw, so the jitter matches the reference
pub(crate) struct FixedSample {
    pub sample: u64,
    pub draws: usize,
}

impl FixedSample {
    pub(crate) const fn new(sample: u64) -> Self {
        Self { sample, draws: 0 }
    }
}

impl TryRng for FixedSample {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        Ok(1)
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        self.draws = self.draws.saturating_add(1);
        Ok(self.sample.wrapping_shl(11) | 0x07FF)
    }

    fn try_fill_bytes(&mut self, _destination: &mut [u8]) -> Result<(), Infallible> {
        Ok(())
    }
}

pub(crate) fn settings(min_timeout: f64) -> BackoffSettings {
    BackoffSettings {
        initial_retry_delay: DEFAULT_INITIAL_RETRY_DELAY,
        max_retry_delay: DEFAULT_MAX_RETRY_DELAY,
        jitter: DEFAULT_JITTER,
        min_timeout,
    }
}

pub(crate) fn availability(healthy: usize, all: usize) -> Availability {
    Availability {
        healthy_deployments: healthy,
        all_deployments: all,
        context_window_fallbacks: None,
        content_policy_fallbacks: None,
        regular_fallbacks: None,
    }
}

pub(crate) fn every_field() -> RetryPolicy {
    RetryPolicy {
        bad_request: Some(1),
        authentication: Some(2),
        timeout: Some(3),
        rate_limit: Some(4),
        content_policy: Some(5),
        internal_server: Some(6),
        service_unavailable: Some(7),
        not_found: Some(8),
        fallback: Some(9),
    }
}

pub(crate) fn seconds(value: f64) -> String {
    format!("{value:.6}")
}

fn load_error(source: &str) -> Option<String> {
    betterlite_config::loader::ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .err()
    .map(|error| error.to_string())
}

#[rstest]
fn replays_the_retry_after_corpus(#[files("fuzz/corpus/retry_after/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let outcome = parse_retry_after(&bytes);
    if name.contains("invalid") {
        assert!(outcome.is_err(), "{name} must not parse");
    } else {
        assert!(outcome.is_ok(), "{name} must parse");
    }
}

#[test]
fn config_corpus_seeds_reach_the_retry_policy_section() {
    assert!(
        load_error(CONFIG_VALID).is_none(),
        "the valid seed must load into the retry policy section"
    );
    let error = load_error(CONFIG_INVALID).expect("the invalid seed must fail");
    assert!(
        error.contains("router_settings.retry_policy"),
        "the invalid seed fails in the retry policy section, got {error}"
    );
}
