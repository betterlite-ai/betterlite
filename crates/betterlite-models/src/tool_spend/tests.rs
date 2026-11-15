#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use rust_decimal::Decimal;

use super::{ToolUsageRequest, build_tool_usage_transaction, decode_tool_spend_time};

fn request<'request>(
    mcp_namespaced_tool_name: Option<&'request str>,
    start_time_iso: &'request str,
) -> ToolUsageRequest<'request> {
    ToolUsageRequest {
        request_id: "req-1",
        start_time_iso,
        mcp_namespaced_tool_name,
        spend: Decimal::new(125, 2),
        total_tokens: 42,
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn deduplicates_names_in_first_seen_order() {
    let request = request(Some("  mcp__fs  "), "2026-01-02T03:04:05Z");
    let transaction = build_tool_usage_transaction(
        &request,
        &names(&["mcp__fs", "search"]),
        &names(&["search", "fetch"]),
    )
    .unwrap();
    assert_eq!(transaction.tool_names, ["mcp__fs", "search", "fetch"]);
    assert_eq!(transaction.request_id, "req-1");
    assert_eq!(transaction.date, "2026-01-02");
}

#[test]
fn every_invoked_tool_keeps_the_full_request_amount() {
    let request = request(None, "2026-01-02T03:04:05Z");
    let transaction =
        build_tool_usage_transaction(&request, &names(&["search", "fetch"]), &[]).unwrap();
    assert_eq!(transaction.spend, Decimal::new(125, 2));
    assert_eq!(transaction.total_tokens, 42);
}

#[test]
fn declared_only_tools_never_count() {
    assert!(
        build_tool_usage_transaction(&request(None, "2026-01-02T03:04:05Z"), &[], &[]).is_none()
    );
    assert!(
        build_tool_usage_transaction(&request(Some("  "), "2026-01-02T03:04:05Z"), &[], &[])
            .is_none()
    );
    assert!(
        build_tool_usage_transaction(&request(Some(""), "2026-01-02T03:04:05Z"), &[], &[])
            .is_none()
    );
    let request = request(Some("   "), "2026-01-02T03:04:05Z");
    let transaction = build_tool_usage_transaction(&request, &names(&["search"]), &[]).unwrap();
    assert_eq!(transaction.tool_names, ["search"]);
}

#[test]
fn keeps_the_original_day_across_an_offset() {
    let request = request(None, "2026-01-02T00:30:00+02:00");
    let transaction = build_tool_usage_transaction(&request, &names(&["search"]), &[]).unwrap();
    assert_eq!(transaction.date, "2026-01-02");
    assert_eq!(
        transaction.start_time.to_rfc3339(),
        "2026-01-01T22:30:00+00:00"
    );
}

#[test]
fn assumes_utc_for_a_naive_time() {
    let request = request(None, "2026-01-02T03:04:05");
    let transaction = build_tool_usage_transaction(&request, &names(&["search"]), &[]).unwrap();
    assert_eq!(
        transaction.start_time.to_rfc3339(),
        "2026-01-02T03:04:05+00:00"
    );
}

#[test]
fn reads_a_z_suffix_and_fractional_seconds() {
    let request = request(None, "2026-01-02T03:04:05.250Z");
    let transaction = build_tool_usage_transaction(&request, &names(&["search"]), &[]).unwrap();
    assert_eq!(transaction.start_time.timestamp_subsec_micros(), 250_000);
    assert_eq!(transaction.date, "2026-01-02");
}

#[rstest]
#[case::words("not-a-time")]
#[case::month_thirteen("2026-13-01T00:00:00")]
#[case::hour_twenty_five("2026-01-02T25:00:00")]
#[case::empty("")]
#[case::trailing_junk("2026-01-02T03:04:05Z junk")]
fn invalid_time_yields_no_transaction(#[case] start_time_iso: &str) {
    let request = request(None, start_time_iso);
    assert!(build_tool_usage_transaction(&request, &names(&["search"]), &[]).is_none());
}

#[test]
fn a_date_alone_starts_at_midnight() {
    let decoded = decode_tool_spend_time("2026-01-02").unwrap();
    assert_eq!(decoded.date, "2026-01-02");
    assert_eq!(decoded.start_time.to_rfc3339(), "2026-01-02T00:00:00+00:00");
}

#[rstest]
fn replays_the_tool_spend_time_corpus(#[files("fuzz/corpus/tool_spend_time/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    drop(decode_tool_spend_time(&text));
}
