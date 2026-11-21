#![cfg(test)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_auth::rejection_provider::ProviderResolution;
use betterlite_error::proxy_rate_limit_error::RateLimitResetWindow;
use betterlite_error::rate_limit::{ProxyRateLimitError, RateLimitCategory, RateLimitDimension};
use betterlite_models::resolution::{ProviderIdentity, ResolutionError, ResolvedModel};
use rstest::rstest;

use crate::limiter_records::{
    RateLimitDescriptor, RateLimitResponse, RateLimitStatus, RateLimitType,
};
use crate::provider_attribution::{
    ModelGroupLimits, PriorityKind, PriorityReservation, PriorityValue, RateLimitErrorAttribution,
    convert_priority_to_percent, handle_rate_limit_error,
};

struct Native;

impl ProviderResolution for Native {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, ResolutionError> {
        if model != "openai/gpt-4o-mini" {
            return Err(ResolutionError::MissingProvider {
                model: model.to_owned(),
            });
        }
        Ok(ResolvedModel {
            model: String::from("gpt-4o-mini"),
            provider: ProviderIdentity::Custom(String::from("openai")),
            api_base: None,
            api_key: None,
        })
    }
}

fn moment(seconds: u64) -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .expect("the moment fits a SystemTime")
}

fn reservation(kind: PriorityKind, value: f64) -> PriorityValue {
    PriorityValue::Reservation(PriorityReservation { kind, value })
}

fn limits(rpm: Option<i64>, tpm: Option<i64>) -> ModelGroupLimits {
    ModelGroupLimits { rpm, tpm }
}

#[test]
fn provider_attribution_tests_behavior_passes_a_plain_number_through() {
    assert_eq!(
        convert_priority_to_percent(PriorityValue::Percent(0.9), None).to_bits(),
        0.9_f64.to_bits()
    );
}

#[test]
fn provider_attribution_tests_behavior_passes_a_percent_reservation_through() {
    let model_info = limits(Some(100), Some(100));
    assert_eq!(
        convert_priority_to_percent(reservation(PriorityKind::Percent, 0.5), Some(&model_info))
            .to_bits(),
        0.5_f64.to_bits()
    );
}

#[test]
fn provider_attribution_tests_behavior_divides_an_rpm_value_by_the_group_rpm() {
    let model_info = limits(Some(1_000), None);
    assert_eq!(
        convert_priority_to_percent(reservation(PriorityKind::Rpm, 900.0), Some(&model_info))
            .to_bits(),
        0.9_f64.to_bits()
    );
}

#[test]
fn provider_attribution_tests_behavior_divides_a_tpm_value_by_the_group_tpm() {
    let model_info = limits(None, Some(1_000_000));
    assert_eq!(
        convert_priority_to_percent(reservation(PriorityKind::Tpm, 900_000.0), Some(&model_info))
            .to_bits(),
        0.9_f64.to_bits()
    );
}

#[rstest]
#[case::rpm_absent(PriorityKind::Rpm, 900.0_f64, None)]
#[case::rpm_zero(PriorityKind::Rpm, 900.0_f64, Some(limits(Some(0), None)))]
#[case::rpm_negative(PriorityKind::Rpm, 900.0_f64, Some(limits(Some(-5), None)))]
#[case::rpm_missing(PriorityKind::Rpm, 900.0_f64, Some(limits(None, Some(1_000))))]
#[case::tpm_absent(PriorityKind::Tpm, 900_000.0_f64, None)]
#[case::tpm_zero(PriorityKind::Tpm, 900_000.0_f64, Some(limits(None, Some(0))))]
#[case::tpm_missing(PriorityKind::Tpm, 900_000.0_f64, Some(limits(Some(1_000), None)))]
fn provider_attribution_tests_behavior_keeps_the_value_when_the_capacity_is_unusable(
    #[case] kind: PriorityKind,
    #[case] value: f64,
    #[case] model_info: Option<ModelGroupLimits>,
) {
    assert_eq!(
        convert_priority_to_percent(reservation(kind, value), model_info.as_ref()).to_bits(),
        value.to_bits()
    );
}

const QUARTER_PERCENT: PriorityValue = PriorityValue::Reservation(PriorityReservation {
    kind: PriorityKind::Percent,
    value: 0.25,
});
const FULL_PERCENT: PriorityValue = PriorityValue::Reservation(PriorityReservation {
    kind: PriorityKind::Percent,
    value: 1.0,
});
const ONE_RPM: PriorityValue = PriorityValue::Reservation(PriorityReservation {
    kind: PriorityKind::Rpm,
    value: 1.0,
});
const TPM_BURST: PriorityValue = PriorityValue::Reservation(PriorityReservation {
    kind: PriorityKind::Tpm,
    value: 900_000.0,
});

#[rstest]
#[case::plain_number("0.9", PriorityValue::Percent(0.9))]
#[case::percent(r#"{"type": "percent", "value": 0.25}"#, QUARTER_PERCENT)]
#[case::missing_type(r#"{"value": 0.25}"#, QUARTER_PERCENT)]
#[case::null_type(r#"{"type": null, "value": 0.25}"#, QUARTER_PERCENT)]
#[case::missing_value(r#"{"type": "rpm"}"#, ONE_RPM)]
#[case::empty_mapping("{}", FULL_PERCENT)]
#[case::tpm(r#"{"type": "tpm", "value": 900000}"#, TPM_BURST)]
#[case::whole_number("1", PriorityValue::Percent(1.0))]
fn provider_attribution_tests_behavior_reads_the_reservation_mapping(
    #[case] text: &str,
    #[case] expected: PriorityValue,
) {
    assert_eq!(
        serde_json::from_str::<PriorityValue>(text).expect("the mapping is a priority value"),
        expected
    );
}

#[rstest]
#[case::plain_number("1e400")]
#[case::reservation_value(r#"{"type": "rpm", "value": 1e400}"#)]
fn provider_attribution_tests_behavior_refuses_a_number_no_float_can_hold(#[case] text: &str) {
    let error = serde_json::from_str::<PriorityValue>(text)
        .expect_err("a number no float can hold is refused");
    assert!(
        error
            .to_string()
            .contains("the priority value is not a finite number"),
        "{error}"
    );
}

#[test]
fn provider_attribution_tests_behavior_takes_the_value_for_an_unknown_reservation_type() {
    let value: PriorityValue = serde_json::from_str(r#"{"type": "kilowatts", "value": 0.25}"#)
        .expect("the mapping parses");
    let model_info = limits(Some(10), Some(10));
    assert_eq!(
        convert_priority_to_percent(value, Some(&model_info)).to_bits(),
        0.25_f64.to_bits()
    );
}

fn status(kind: RateLimitType) -> RateLimitStatus {
    RateLimitStatus {
        code: String::from("OVER_LIMIT"),
        current_limit: 100,
        limit_remaining: -3,
        rate_limit_type: kind,
        descriptor_key: String::from("model_per_project_itpm"),
        descriptor_value: Some(String::from("proj:gpt-4o")),
    }
}

fn response(statuses: Vec<RateLimitStatus>) -> RateLimitResponse {
    RateLimitResponse {
        overall_code: String::from("OVER_LIMIT"),
        statuses,
        reservation_windows: None,
    }
}

fn descriptor(key: &str, value: &str) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: String::from(key),
        value: String::from(value),
        rate_limit: None,
    }
}

fn window() -> RateLimitResetWindow {
    RateLimitResetWindow {
        now: moment(1_718_452_800),
        window_size: 60,
    }
}

fn attribution<'native>(
    native: &'native Native,
    model: Option<&'native str>,
) -> RateLimitErrorAttribution<'native> {
    RateLimitErrorAttribution {
        requested_model: model,
        native,
        router: None,
    }
}

fn projected(
    response: &RateLimitResponse,
    descriptors: &[RateLimitDescriptor],
    native: &Native,
    model: Option<&str>,
) -> ProxyRateLimitError {
    handle_rate_limit_error(response, descriptors, window(), &attribution(native, model))
        .expect("the window is representable")
        .expect("an OVER_LIMIT status raises")
}

#[test]
fn provider_attribution_tests_behavior_raises_nothing_without_an_over_limit_status() {
    let native = Native;
    let ok = RateLimitStatus {
        code: String::from("OK"),
        ..status(RateLimitType::Tokens)
    };
    let without_status = handle_rate_limit_error(
        &response(vec![ok]),
        &[],
        window(),
        &attribution(&native, None),
    )
    .expect("the window is representable");
    assert!(without_status.is_none());
    let without_statuses = handle_rate_limit_error(
        &response(Vec::new()),
        &[],
        window(),
        &attribution(&native, None),
    )
    .expect("the window is representable");
    assert!(without_statuses.is_none());
}

#[test]
fn provider_attribution_tests_behavior_takes_the_first_over_limit_status() {
    let native = Native;
    let earlier = RateLimitStatus {
        code: String::from("OK"),
        descriptor_key: String::from("key_alias"),
        ..status(RateLimitType::Tokens)
    };
    let later = RateLimitStatus {
        descriptor_key: String::from("key_alias"),
        descriptor_value: Some(String::from("later")),
        rate_limit_type: RateLimitType::MaxParallelRequests,
        ..status(RateLimitType::Tokens)
    };
    let error = projected(
        &response(vec![earlier, status(RateLimitType::Tokens), later]),
        &[
            descriptor("key_alias", "first"),
            descriptor("model_per_project_itpm", "proj:gpt-4o"),
            descriptor("key_alias", "later"),
        ],
        &native,
        None,
    );
    assert!(error.message.starts_with(
        "Rate limit exceeded for model_per_project_itpm: proj:gpt-4o. Limit type: tokens. "
    ));
    let headers = error
        .headers
        .as_ref()
        .expect("the projection sets its headers");
    assert_eq!(headers["rate_limit_type"], "tokens");
}

#[test]
fn provider_attribution_tests_behavior_matches_the_descriptor_by_key_and_value() {
    let native = Native;
    let error = projected(
        &response(vec![status(RateLimitType::Tokens)]),
        &[
            descriptor("model_per_project_itpm", "other:model"),
            descriptor("model_per_project_itpm", "proj:gpt-4o"),
        ],
        &native,
        None,
    );
    assert!(
        error
            .message
            .starts_with("Rate limit exceeded for model_per_project_itpm: proj:gpt-4o. ")
    );
}

#[test]
fn provider_attribution_tests_behavior_names_unknown_when_no_descriptor_matches() {
    let native = Native;
    let error = projected(
        &response(vec![status(RateLimitType::Tokens)]),
        &[descriptor("other_key", "proj:gpt-4o")],
        &native,
        None,
    );
    assert!(
        error
            .message
            .starts_with("Rate limit exceeded for model_per_project_itpm: unknown. ")
    );
}

#[test]
fn provider_attribution_tests_behavior_matches_a_status_without_a_descriptor_value() {
    let native = Native;
    let without_value = RateLimitStatus {
        descriptor_value: None,
        ..status(RateLimitType::Tokens)
    };
    let error = projected(
        &response(vec![without_value]),
        &[
            descriptor("model_per_project_itpm", "first"),
            descriptor("model_per_project_itpm", "second"),
        ],
        &native,
        None,
    );
    assert!(
        error
            .message
            .starts_with("Rate limit exceeded for model_per_project_itpm: first. ")
    );
}

#[test]
fn provider_attribution_tests_behavior_carries_the_headers_and_the_provider() {
    let native = Native;
    let error = projected(
        &response(vec![status(RateLimitType::Tokens)]),
        &[descriptor("model_per_project_itpm", "proj:gpt-4o")],
        &native,
        Some("openai/gpt-4o-mini"),
    );
    let headers = error
        .headers
        .as_ref()
        .expect("the projection sets its headers");
    assert_eq!(headers["retry-after"], "60");
    assert_eq!(headers["rate_limit_type"], "tokens");
    assert_eq!(headers["reset_at"], "2024-06-15 12:01:00 UTC");
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
    assert_eq!(error.category, RateLimitCategory::LitellmRateLimit);
    assert_eq!(error.model, "gpt-4o-mini");
    assert_eq!(error.llm_provider, "openai");
    assert!(
        error
            .message
            .ends_with("Limit type: tokens. Current limit: 100, Remaining: 0. Limit resets at: 2024-06-15 12:01:00 UTC")
    );
}

#[test]
fn provider_attribution_tests_behavior_maps_max_parallel_requests_to_concurrent_requests() {
    let native = Native;
    let error = projected(
        &response(vec![status(RateLimitType::MaxParallelRequests)]),
        &[descriptor("model_per_project_itpm", "proj:gpt-4o")],
        &native,
        None,
    );
    assert_eq!(
        error.rate_limit_type,
        Some(RateLimitDimension::ConcurrentRequests)
    );
    let headers = error
        .headers
        .as_ref()
        .expect("the projection sets its headers");
    assert_eq!(headers["rate_limit_type"], "max_parallel_requests");
}

#[test]
fn provider_attribution_tests_behavior_falls_back_to_the_proxy_provider() {
    let native = Native;
    let error = projected(
        &response(vec![status(RateLimitType::Tokens)]),
        &[],
        &native,
        Some("unknown-model"),
    );
    assert_eq!(error.model, "unknown-model");
    assert_eq!(error.llm_provider, "litellm_proxy");
    let absent = projected(
        &response(vec![status(RateLimitType::Tokens)]),
        &[],
        &native,
        None,
    );
    assert_eq!(absent.model, "");
    assert_eq!(absent.llm_provider, "litellm_proxy");
}
