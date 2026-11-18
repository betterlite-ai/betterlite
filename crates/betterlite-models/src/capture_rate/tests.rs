#![cfg(test)]

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use super::alert::alert_message;
use super::compute::{CaptureRateError, CaptureRateRequest, compute_capture_rate};
use super::format::{format_percent, format_usd};
use super::report::{CaptureRateDay, CaptureRateReport, CheckResult};
use super::settings::SpendCaptureProvider;

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn date(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

fn days(pairs: &[(&str, &str)]) -> BTreeMap<String, Decimal> {
    pairs
        .iter()
        .map(|(day, spend)| ((*day).to_owned(), dec(spend)))
        .collect()
}

fn request(
    start: &str,
    end: &str,
    captured: (BTreeMap<String, Decimal>, BTreeMap<String, Decimal>),
    threshold: &str,
) -> CaptureRateReport {
    compute_capture_rate(&CaptureRateRequest {
        provider: SpendCaptureProvider::Openai,
        start_date: date(start),
        end_date: date(end),
        captured_by_day: &captured.0,
        billed_by_day: &captured.1,
        threshold: dec(threshold),
    })
    .unwrap()
}

fn captured(
    captured: BTreeMap<String, Decimal>,
    billed: BTreeMap<String, Decimal>,
) -> CaptureRateReport {
    request("2026-11-10", "2026-11-12", (captured, billed), "0.9")
}

#[test]
fn computes_days_over_the_inclusive_window() {
    let report = captured(
        days(&[("2026-11-10", "2.0")]),
        days(&[("2026-11-10", "1.0"), ("2026-11-12", "4.0")]),
    );
    assert_eq!(report.start_date, "2026-11-10");
    assert_eq!(report.end_date, "2026-11-12");
    assert_eq!(report.days.len(), 3);
    assert_eq!(report.days[0].date, "2026-11-10");
    assert_eq!(report.days[0].captured_spend, dec("2.0"));
    assert_eq!(report.days[0].provider_spend, dec("1.0"));
    assert_eq!(report.days[0].capture_rate, Some(dec("2.0")));
    assert_eq!(report.days[1].date, "2026-11-11");
    assert_eq!(report.days[1].captured_spend, Decimal::ZERO);
    assert_eq!(report.days[1].provider_spend, Decimal::ZERO);
    assert_eq!(report.days[1].capture_rate, None);
    assert_eq!(report.days[2].date, "2026-11-12");
    assert_eq!(report.days[2].capture_rate, Some(Decimal::ZERO));
    assert_eq!(report.captured_spend, dec("2.0"));
    assert_eq!(report.provider_spend, dec("5.0"));
    assert_eq!(report.capture_rate, Some(dec("0.4")));
    assert!(report.below_threshold);
}

#[test]
fn a_single_day_window_counts_that_day() {
    let report = request(
        "2026-11-16",
        "2026-11-16",
        (
            days(&[("2026-11-16", "0.5")]),
            days(&[("2026-11-16", "1.0")]),
        ),
        "0.9",
    );
    assert_eq!(report.days.len(), 1);
    assert_eq!(report.capture_rate, Some(dec("0.5")));
    assert!(report.below_threshold);
}

#[test]
fn a_window_that_ends_before_it_starts_has_no_days() {
    let report = request(
        "2026-11-16",
        "2026-11-10",
        (
            days(&[("2026-11-12", "2.0")]),
            days(&[("2026-11-12", "1.0")]),
        ),
        "0.9",
    );
    assert!(report.days.is_empty());
    assert_eq!(report.captured_spend, Decimal::ZERO);
    assert_eq!(report.provider_spend, Decimal::ZERO);
    assert_eq!(report.capture_rate, None);
    assert!(!report.below_threshold);
}

#[test]
fn equality_with_the_threshold_is_healthy() {
    let report = request(
        "2026-11-10",
        "2026-11-10",
        (
            days(&[("2026-11-10", "90")]),
            days(&[("2026-11-10", "100")]),
        ),
        "0.9",
    );
    assert_eq!(report.capture_rate, Some(dec("0.9")));
    assert!(!report.below_threshold);
    assert_eq!(alert_message(&CheckResult::Report(report)), None);
}

#[test]
fn a_rate_above_one_is_not_clamped() {
    let report = captured(days(&[("2026-11-10", "2")]), days(&[("2026-11-10", "1")]));
    assert_eq!(report.capture_rate, Some(dec("2")));
    assert!(!report.below_threshold);
    assert_eq!(report.days[0].capture_rate, Some(dec("2")));
}

#[test]
fn a_zero_or_negative_bill_has_no_ratio() {
    let zero = request(
        "2026-11-10",
        "2026-11-10",
        (days(&[("2026-11-10", "5")]), days(&[("2026-11-10", "0")])),
        "0.9",
    );
    assert_eq!(zero.capture_rate, None);
    assert_eq!(zero.days[0].capture_rate, None);
    assert!(!zero.below_threshold);
    assert_eq!(alert_message(&CheckResult::Report(zero)), None);
    let negative = request(
        "2026-11-10",
        "2026-11-10",
        (days(&[("2026-11-10", "5")]), days(&[("2026-11-10", "-1")])),
        "0.9",
    );
    assert_eq!(negative.capture_rate, None);
    assert!(!negative.below_threshold);
}

#[test]
fn an_overflowing_captured_sum_is_an_error() {
    let maximum = "79228162514264337593543950335";
    let error = compute_capture_rate(&CaptureRateRequest {
        provider: SpendCaptureProvider::Openai,
        start_date: date("2026-11-10"),
        end_date: date("2026-11-11"),
        captured_by_day: &days(&[("2026-11-10", maximum), ("2026-11-11", maximum)]),
        billed_by_day: &days(&[]),
        threshold: dec("0.9"),
    })
    .unwrap_err();
    assert!(matches!(error, CaptureRateError::CapturedOverflow));
}

#[test]
fn an_overflowing_provider_sum_is_an_error() {
    let maximum = "79228162514264337593543950335";
    let error = compute_capture_rate(&CaptureRateRequest {
        provider: SpendCaptureProvider::Openai,
        start_date: date("2026-11-10"),
        end_date: date("2026-11-11"),
        captured_by_day: &days(&[]),
        billed_by_day: &days(&[("2026-11-10", maximum), ("2026-11-11", maximum)]),
        threshold: dec("0.9"),
    })
    .unwrap_err();
    assert!(matches!(error, CaptureRateError::ProviderOverflow));
}

#[test]
fn an_overflowing_ratio_is_an_error() {
    let error = compute_capture_rate(&CaptureRateRequest {
        provider: SpendCaptureProvider::Openai,
        start_date: date("2026-11-10"),
        end_date: date("2026-11-10"),
        captured_by_day: &days(&[("2026-11-10", "79228162514264337593543950335")]),
        billed_by_day: &days(&[("2026-11-10", "0.0000000000000000000000000001")]),
        threshold: dec("0.9"),
    })
    .unwrap_err();
    assert!(matches!(error, CaptureRateError::RatioOverflow));
}

#[test]
fn alert_message_reports_a_missing_credential() {
    let result = CheckResult::CredentialMissing {
        provider: SpendCaptureProvider::Openai,
        env_var: "OPENAI_ADMIN_KEY".to_owned(),
    };
    assert_eq!(
        alert_message(&result).unwrap(),
        "Spend capture-rate check: OPENAI_ADMIN_KEY is not set, so the openai bill cannot be read. Set it or remove general_settings.spend_capture_rate_check. https://docs.litellm.ai/docs/proxy/spend_capture_rate"
    );
}

#[test]
fn alert_message_reports_a_request_failure() {
    let result = CheckResult::RequestFailed {
        provider: SpendCaptureProvider::Openai,
        detail: "HTTP 500: boom".to_owned(),
    };
    assert_eq!(
        alert_message(&result).unwrap(),
        "Spend capture-rate check: could not read the openai bill (HTTP 500: boom). https://docs.litellm.ai/docs/proxy/spend_capture_rate"
    );
}

#[test]
fn alert_message_formats_a_below_threshold_report() {
    let report = request(
        "2026-11-10",
        "2026-11-12",
        (
            days(&[("2026-11-11", "45")]),
            days(&[("2026-11-12", "100")]),
        ),
        "0.9",
    );
    assert_eq!(
        alert_message(&CheckResult::Report(report)).unwrap(),
        "Spend capture rate for openai is 45.0%, under the 90% threshold: LiteLLM captured $45.00 of the $100.00 openai bill for 2026-11-10 to 2026-11-12. Requests reach openai outside LiteLLM or cost tracking is dropping spend. https://docs.litellm.ai/docs/proxy/spend_capture_rate"
    );
}

#[test]
fn alert_message_groups_thousands() {
    let report = request(
        "2026-11-10",
        "2026-11-10",
        (
            days(&[("2026-11-10", "1234567.5")]),
            days(&[("2026-11-10", "2000000")]),
        ),
        "0.9",
    );
    assert_eq!(
        alert_message(&CheckResult::Report(report)).unwrap(),
        "Spend capture rate for openai is 61.7%, under the 90% threshold: LiteLLM captured $1,234,567.50 of the $2,000,000.00 openai bill for 2026-11-10 to 2026-11-10. Requests reach openai outside LiteLLM or cost tracking is dropping spend. https://docs.litellm.ai/docs/proxy/spend_capture_rate"
    );
}

#[test]
fn alert_message_skips_a_flagged_report_without_a_rate() {
    let report = CaptureRateReport {
        provider: SpendCaptureProvider::Openai,
        start_date: "2026-11-10".to_owned(),
        end_date: "2026-11-10".to_owned(),
        captured_spend: dec("1"),
        provider_spend: Decimal::ZERO,
        capture_rate: None,
        threshold: dec("0.9"),
        below_threshold: true,
        days: Vec::new(),
    };
    assert_eq!(alert_message(&CheckResult::Report(report)), None);
}

#[test]
fn serializes_money_as_a_decimal_json_number() {
    let report = request(
        "2026-11-10",
        "2026-11-10",
        (
            days(&[("2026-11-10", "90")]),
            days(&[("2026-11-10", "100")]),
        ),
        "0.9",
    );
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["captured_spend"].to_string(), "90.0");
    assert_eq!(value["provider_spend"].to_string(), "100.0");
    assert_eq!(value["threshold"].to_string(), "0.9");
    assert_eq!(value["capture_rate"].to_string(), "0.9");
    assert_eq!(value["provider"].to_string(), "\"openai\"");
    assert_eq!(value["below_threshold"].to_string(), "false");
    assert_eq!(value["days"][0]["captured_spend"].to_string(), "90.0");
    assert_eq!(value["days"][0]["capture_rate"].to_string(), "0.9");
}

#[test]
fn serializes_a_missing_ratio_as_null() {
    let report = request("2026-11-10", "2026-11-10", (days(&[]), days(&[])), "0.9");
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["capture_rate"].to_string(), "null");
    assert_eq!(value["days"][0]["capture_rate"].to_string(), "null");
    assert_eq!(value["captured_spend"].to_string(), "0.0");
}

#[test]
fn published_rate_and_provider_cover_every_outcome() {
    let report = request(
        "2026-11-10",
        "2026-11-10",
        (days(&[("2026-11-10", "1")]), days(&[("2026-11-10", "1")])),
        "0.9",
    );
    let outcome = CheckResult::Report(report);
    assert_eq!(outcome.provider(), SpendCaptureProvider::Openai);
    assert_eq!(outcome.published_rate(), Some(Decimal::ONE));
    let missing = CheckResult::CredentialMissing {
        provider: SpendCaptureProvider::Openai,
        env_var: "OPENAI_ADMIN_KEY".to_owned(),
    };
    assert_eq!(missing.provider(), SpendCaptureProvider::Openai);
    assert_eq!(missing.published_rate(), None);
    let failed = CheckResult::RequestFailed {
        provider: SpendCaptureProvider::Openai,
        detail: "HTTP 500: boom".to_owned(),
    };
    assert_eq!(failed.provider(), SpendCaptureProvider::Openai);
    assert_eq!(failed.published_rate(), None);
}

#[test]
fn formats_usd_with_separators_and_half_even_rounding() {
    assert_eq!(format_usd(Decimal::ZERO), "0.00");
    assert_eq!(format_usd(dec("45")), "45.00");
    assert_eq!(format_usd(dec("999")), "999.00");
    assert_eq!(format_usd(dec("1000")), "1,000.00");
    assert_eq!(format_usd(dec("1234567.5")), "1,234,567.50");
    assert_eq!(format_usd(dec("1234567890.12")), "1,234,567,890.12");
    assert_eq!(format_usd(dec("-1234.5")), "-1,234.50");
    assert_eq!(format_usd(dec("999.999")), "1,000.00");
    assert_eq!(format_usd(dec("-0.001")), "-0.00");
    assert_eq!(format_usd(dec("0.005")), "0.00");
    assert_eq!(format_usd(dec("0.015")), "0.02");
}

#[test]
fn formats_percent_with_half_even_rounding() {
    assert_eq!(format_percent(dec("0"), 1), "0.0%");
    assert_eq!(format_percent(dec("0.45"), 1), "45.0%");
    assert_eq!(format_percent(dec("0.899"), 1), "89.9%");
    assert_eq!(format_percent(dec("1"), 1), "100.0%");
    assert_eq!(format_percent(dec("2"), 1), "200.0%");
    assert_eq!(format_percent(dec("-0.1"), 1), "-10.0%");
    assert_eq!(format_percent(dec("0.9"), 0), "90%");
    assert_eq!(format_percent(dec("1"), 0), "100%");
    assert_eq!(format_percent(dec("0.05"), 0), "5%");
}
