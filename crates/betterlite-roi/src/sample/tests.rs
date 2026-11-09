#![cfg(test)]

use chrono::{DateTime, Days, NaiveDate, Utc};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::records::ROIEstimateStatus;
use crate::sample::sample_report;
use crate::settings::DEFAULT_PROMPT;
use crate::summary::summarize;

const NOW: &str = "2026-03-15T10:30:00+00:00";

fn instant(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

#[rstest]
#[case::alex(0, "alex", &["alex@example.com"], "Add usage breakdown by model", 6.5_f64)]
#[case::jordan(1, "jordan", &["jordan@example.com"], "Fix streaming response cancellation", 4.0_f64)]
#[case::casey(2, "casey", &[], "Add integration tests for billing", 5.5_f64)]
fn sample_report_fills_each_example(
    #[case] index: usize,
    #[case] login: &str,
    #[case] emails: &[&str],
    #[case] title: &str,
    #[case] hours: f64,
) {
    let report = sample_report(instant(NOW));
    let pull = &report.pulls[index];
    assert_eq!(pull.repo, "example/gateway");
    assert_eq!(pull.login, login);
    assert_eq!(pull.title, title);
    assert_eq!(pull.url, "");
    let recorded: Vec<&str> = pull.emails.iter().map(String::as_str).collect();
    assert_eq!(recorded, emails);
    assert_eq!(
        pull.profile_email,
        emails.first().copied().unwrap_or_default()
    );
    assert_eq!(pull.estimate.hours, Some(hours));
}

#[rstest]
#[case::alex(0, 142, 47, 12)]
#[case::jordan(1, 143, 70, 16)]
#[case::casey(2, 144, 93, 20)]
fn sample_report_numbers_pulls_by_index(
    #[case] index: usize,
    #[case] number: i64,
    #[case] additions: i64,
    #[case] deletions: i64,
) {
    let report = sample_report(instant(NOW));
    let pull = &report.pulls[index];
    assert_eq!(pull.number, number);
    assert_eq!(pull.additions, additions);
    assert_eq!(pull.deletions, deletions);
    assert_eq!(pull.head_sha, format!("sample-{index}"));
    assert_eq!(pull.changed_files, 3);
    assert_eq!(pull.commit_count, 1);
    assert!(!pull.incomplete_metadata);
    assert!(pull.cache_key.is_none());
    assert!(pull.commit_emails.is_none());
}

#[rstest]
#[case::alex(0)]
#[case::jordan(1)]
#[case::casey(2)]
fn sample_report_marks_every_pull_estimated(#[case] index: usize) {
    let report = sample_report(instant(NOW));
    let estimate = &report.pulls[index].estimate;
    assert_eq!(estimate.status, ROIEstimateStatus::Estimated);
    assert_eq!(
        estimate.reasoning,
        "Sample estimate of engineering effort without AI assistance. Live estimates use PR descriptions, file change counts, and commit metadata."
    );
    assert_eq!(estimate.model.as_deref(), Some("your-estimator-model"));
    assert_eq!(estimate.effort_basis.as_deref(), Some("without_ai"));
    assert_eq!(estimate.evidence_source.as_deref(), Some("pr_metadata"));
    assert_eq!(estimate.cached, Some(false));
}

#[rstest]
#[case::mid_month("2026-03-15T10:30:00+00:00", "2026-02-14", "2026-03-15")]
#[case::month_start("2026-03-01T00:00:00+00:00", "2026-01-31", "2026-03-01")]
#[case::leap_year("2024-03-01T00:00:00+00:00", "2024-02-01", "2024-03-01")]
#[case::year_start("2026-01-10T00:00:00+00:00", "2025-12-12", "2026-01-10")]
fn sample_report_dates_follow_now(#[case] now: &str, #[case] start: &str, #[case] end: &str) {
    let report = sample_report(instant(now));
    assert_eq!(report.start, start);
    assert_eq!(report.end, end);
}

#[rstest]
#[case::alex(0, 2, "2026-02-16")]
#[case::jordan(1, 4, "2026-02-18")]
#[case::casey(2, 6, "2026-02-20")]
fn sample_report_merges_a_few_days_after_start(
    #[case] index: usize,
    #[case] offset: u64,
    #[case] day: &str,
) {
    let report = sample_report(instant(NOW));
    let start = NaiveDate::parse_from_str(&report.start, "%Y-%m-%d").unwrap();
    let expected = start.checked_add_days(Days::new(offset)).unwrap();
    assert_eq!(
        report.pulls[index].merged_at,
        format!("{expected}T14:20:00Z")
    );
    assert_eq!(report.pulls[index].merged_at, format!("{day}T14:20:00Z"));
}

#[rstest]
#[case::whole_second("2026-03-15T10:30:00+00:00", "2026-03-15T10:30:00+00:00")]
#[case::microseconds("2026-03-15T10:30:00.123456+00:00", "2026-03-15T10:30:00.123456+00:00")]
#[case::trailing_zeros("2026-03-15T10:30:00.100000+00:00", "2026-03-15T10:30:00.100000+00:00")]
fn sample_report_prints_synced_at_as_python_isoformat(#[case] now: &str, #[case] synced_at: &str) {
    assert_eq!(sample_report(instant(now)).synced_at, synced_at);
}

#[test]
fn sample_report_leaves_casey_without_spend() {
    let report = sample_report(instant(NOW));
    assert_eq!(report.pulls.len(), 3);
    let casey = &report.pulls[2];
    assert!(casey.emails.is_empty());
    assert_eq!(casey.profile_email, "");
    assert_eq!(report.spend.len(), 2);
    let spenders: Vec<&str> = report
        .spend
        .iter()
        .map(|row| row.user_id.as_str())
        .collect();
    assert_eq!(spenders, ["alex", "jordan"]);
}

#[rstest]
#[case::alex(0, "alex", "alex@example.com", "2026-02-16")]
#[case::jordan(1, "jordan", "jordan@example.com", "2026-02-18")]
fn sample_report_spends_each_matched_example(
    #[case] index: usize,
    #[case] login: &str,
    #[case] email: &str,
    #[case] date: &str,
) {
    let report = sample_report(instant(NOW));
    let row = &report.spend[index];
    assert_eq!(row.user_id, login);
    assert_eq!(row.email, email);
    assert_eq!(row.date, date);
    let merged_day: String = report.pulls[index].merged_at.chars().take(10).collect();
    assert_eq!(row.date, merged_day);
    assert_eq!(row.requests, 150);
}

#[rstest]
#[case::alex(0, "18.2")]
#[case::jordan(1, "12.8")]
fn sample_report_keeps_spend_exact(#[case] index: usize, #[case] text: &str) {
    let report = sample_report(instant(NOW));
    let spend = report.spend[index].spend;
    assert_eq!(spend.to_string(), text);
    assert_eq!(spend, Decimal::from_str_exact(text).unwrap());
}

#[test]
fn sample_report_totals_spend_without_float_drift() {
    let report = sample_report(instant(NOW));
    let total = report.spend[0]
        .spend
        .checked_add(report.spend[1].spend)
        .unwrap();
    assert_eq!(total.to_string(), "31.0");
}

#[test]
fn sample_report_carries_the_demo_settings() {
    let report = sample_report(instant(NOW));
    assert_eq!(report.mode, "demo");
    assert_eq!(report.repos, vec!["example/gateway"]);
    assert_eq!(report.estimator_model, "your-estimator-model");
    assert_eq!(report.estimator_prompt, DEFAULT_PROMPT);
    assert_eq!(
        report.estimator_prompt,
        "Estimate how many hours it would take an engineer to complete the work in this pull request without AI assistance. Explain your estimate briefly."
    );
    assert_eq!(report.effort_basis, "without_ai");
    assert_eq!(report.settings_fingerprint, "sample");
    assert!(report.warnings.is_none());
    assert!(report.unavailable_repos.is_none());
    assert!(report.id.is_none());
}

#[test]
fn sample_report_summarizes_without_error() {
    let report = sample_report(instant(NOW));
    let summary = summarize(&report, &IndexMap::new()).unwrap();
    assert_eq!(summary.pulls.len(), 3);
    assert_eq!(
        summary.pulls.first().map(|pull| pull.login.as_str()),
        Some("casey")
    );
    assert_eq!(summary.metrics.merged_prs, 3);
    assert_eq!(summary.metrics.estimated_prs, 3);
    assert_eq!(summary.metrics.matched_prs, 2);
    assert_eq!(summary.metrics.cohort_people, 2);
    assert_eq!(summary.metrics.people_with_prs, 3);
    assert_eq!(summary.metrics.matched_spend, Decimal::new(310, 1));
    assert_eq!(summary.metrics.total_spend, Decimal::new(310, 1));
    assert_eq!(summary.metrics.output_hours, Decimal::new(105, 1));
    assert_eq!(summary.metrics.total_output_hours, Decimal::new(160, 1));
}

#[test]
fn sample_report_clamps_dates_before_the_calendar() {
    let report = sample_report(DateTime::<Utc>::MIN_UTC);
    assert_eq!(report.start, "0001-01-01");
    assert_eq!(report.end, "0001-01-01");
    assert_eq!(report.pulls[0].merged_at, "0001-01-01T14:20:00Z");
    assert_eq!(report.pulls[2].merged_at, "0001-01-01T14:20:00Z");
    assert_eq!(report.spend[0].date, "0001-01-01");
    assert_eq!(report.spend[1].date, "0001-01-01");
}

#[test]
fn sample_report_clamps_a_year_one_start() {
    let report = sample_report(instant("0001-01-05T00:00:00+00:00"));
    assert_eq!(report.start, "0001-01-01");
    assert_eq!(report.end, "0001-01-05");
    assert_eq!(report.pulls[0].merged_at, "0001-01-01T14:20:00Z");
}
