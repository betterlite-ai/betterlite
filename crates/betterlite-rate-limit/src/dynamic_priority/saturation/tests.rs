#![cfg(test)]

use super::check_model_saturation;
use crate::dynamic_priority::settings::{PriorityPolicy, PrioritySettings};
use crate::dynamic_priority::tests::support::{CacheRead, RecordingCache};
use crate::limiter_records::CacheCounterValue;
use crate::provider_attribution::ModelGroupLimits;

const RPM_KEY: &str = "{model_saturation_check:gpt-4}:requests";
const TPM_KEY: &str = "{model_saturation_check:gpt-4}:tokens";

fn text(value: &str) -> CacheCounterValue {
    CacheCounterValue::Text(value.to_owned())
}

const fn limits(rpm: Option<i64>, tpm: Option<i64>) -> ModelGroupLimits {
    ModelGroupLimits { rpm, tpm }
}

fn read(key: &str) -> CacheRead {
    CacheRead {
        key: key.to_owned(),
        local_only: false,
        local_ttl: 60,
    }
}

async fn saturation(
    values: &[(&str, CacheCounterValue)],
    limits: ModelGroupLimits,
) -> (f64, Vec<CacheRead>) {
    let cache = RecordingCache::seeded(values);
    let saturation =
        check_model_saturation("gpt-4", &limits, &cache, &PriorityPolicy::default()).await;
    (saturation, cache.reads())
}

#[tokio::test]
async fn no_limits_read_nothing() {
    let (saturation, reads) = saturation(&[(RPM_KEY, text("900"))], limits(None, None)).await;
    assert_eq!(saturation.to_bits(), 0.0_f64.to_bits());
    assert!(reads.is_empty());
}

#[tokio::test]
async fn non_positive_limits_read_nothing() {
    let (saturation, reads) = saturation(
        &[(RPM_KEY, text("900")), (TPM_KEY, text("900"))],
        limits(Some(0), Some(-5)),
    )
    .await;
    assert_eq!(saturation.to_bits(), 0.0_f64.to_bits());
    assert!(reads.is_empty());
}

#[tokio::test]
async fn a_zero_token_limit_reads_nothing() {
    let (saturation, reads) = saturation(&[(TPM_KEY, text("900"))], limits(None, Some(0))).await;
    assert_eq!(saturation.to_bits(), 0.0_f64.to_bits());
    assert!(reads.is_empty());
}

#[tokio::test]
async fn rpm_saturation_is_the_request_ratio() {
    let (saturation, reads) = saturation(&[(RPM_KEY, text("50"))], limits(Some(100), None)).await;
    assert_eq!(saturation.to_bits(), 0.5_f64.to_bits());
    assert_eq!(reads, vec![read(RPM_KEY)]);
}

#[tokio::test]
async fn tpm_saturation_is_the_token_ratio() {
    let (saturation, reads) = saturation(&[(TPM_KEY, text("900"))], limits(None, Some(1000))).await;
    assert_eq!(saturation.to_bits(), 0.9_f64.to_bits());
    assert_eq!(reads, vec![read(TPM_KEY)]);
}

#[tokio::test]
async fn the_larger_ratio_wins() {
    let values = [(RPM_KEY, text("50")), (TPM_KEY, text("900"))];
    let (tokens_win, reads) = saturation(&values, limits(Some(100), Some(1000))).await;
    assert_eq!(tokens_win.to_bits(), 0.9_f64.to_bits());
    assert_eq!(reads, vec![read(RPM_KEY), read(TPM_KEY)]);
    let values = [(RPM_KEY, text("90")), (TPM_KEY, text("500"))];
    let (requests_win, _) = saturation(&values, limits(Some(100), Some(1000))).await;
    assert_eq!(requests_win.to_bits(), 0.9_f64.to_bits());
}

#[tokio::test]
async fn a_missing_counter_reads_as_no_saturation() {
    let (saturation, reads) = saturation(&[], limits(Some(100), None)).await;
    assert_eq!(saturation.to_bits(), 0.0_f64.to_bits());
    assert_eq!(reads, vec![read(RPM_KEY)]);
}

#[tokio::test]
async fn a_requests_counter_that_is_not_whole_aborts_the_check() {
    let (saturation, reads) = saturation(
        &[(RPM_KEY, text("abc")), (TPM_KEY, text("900"))],
        limits(Some(100), Some(1000)),
    )
    .await;
    assert_eq!(saturation.to_bits(), 0.0_f64.to_bits());
    assert_eq!(reads, vec![read(RPM_KEY)]);
}

#[tokio::test]
async fn a_tokens_counter_that_is_not_a_number_aborts_the_check() {
    let (saturation, _) = saturation(
        &[(RPM_KEY, text("50")), (TPM_KEY, text("abc"))],
        limits(Some(100), Some(1000)),
    )
    .await;
    assert_eq!(saturation.to_bits(), 0.0_f64.to_bits());
}

#[tokio::test]
async fn a_fractional_requests_counter_aborts_the_check() {
    let (saturation, _) = saturation(&[(RPM_KEY, text("12.5"))], limits(Some(100), None)).await;
    assert_eq!(saturation.to_bits(), 0.0_f64.to_bits());
}

#[tokio::test]
async fn a_fractional_tokens_counter_still_counts() {
    let (saturation, _) = saturation(&[(TPM_KEY, text("12.5"))], limits(None, Some(25))).await;
    assert_eq!(saturation.to_bits(), 0.5_f64.to_bits());
}

#[tokio::test]
async fn a_float_requests_counter_truncates_toward_zero() {
    let (saturation, _) = saturation(
        &[(RPM_KEY, CacheCounterValue::Float(12.9))],
        limits(Some(10), None),
    )
    .await;
    assert_eq!(saturation.to_bits(), 1.2_f64.to_bits());
}

#[tokio::test]
async fn a_float_tokens_counter_keeps_its_fraction() {
    let (saturation, _) = saturation(
        &[(TPM_KEY, CacheCounterValue::Float(12.5))],
        limits(None, Some(25)),
    )
    .await;
    assert_eq!(saturation.to_bits(), 0.5_f64.to_bits());
}

#[tokio::test]
async fn the_configured_ttl_reaches_every_read() {
    let cache = RecordingCache::seeded(&[(RPM_KEY, text("50"))]);
    let policy = PriorityPolicy {
        settings: PrioritySettings {
            default_priority: 0.25,
            saturation_threshold: 0.50,
            saturation_check_cache_ttl: 120,
        },
        reservation: None,
    };
    let saturation =
        check_model_saturation("gpt-4", &limits(Some(100), None), &cache, &policy).await;
    assert_eq!(saturation.to_bits(), 0.5_f64.to_bits());
    assert_eq!(
        cache.reads(),
        vec![CacheRead {
            key: RPM_KEY.to_owned(),
            local_only: false,
            local_ttl: 120,
        }]
    );
}
