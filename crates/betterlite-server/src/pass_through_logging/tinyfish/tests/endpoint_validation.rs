#![cfg(test)]

use std::time::Duration;

use rstest::rstest;

use super::{
    TINYFISH_ENVELOPE_FIELDS, TINYFISH_MAX_POLL_FAILURES, TINYFISH_MAX_POLL_SECONDS,
    TINYFISH_POLL_SECONDS, TINYFISH_RUN_AUTH_FIELDS, TINYFISH_TERMINAL_STATUSES,
    TINYFISH_TIMEOUT_SECONDS, is_allowed_tinyfish_endpoint,
};

#[rstest]
#[case::run_submit("POST", "/v1/automation/run", true)]
#[case::run_async_submit("POST", "/v1/automation/run-async", true)]
#[case::run_sse_submit("POST", "/v1/automation/run-sse", true)]
#[case::run_status("GET", "/v1/runs/run-1", true)]
#[case::cancel_run("POST", "/v1/runs/run-1/cancel", true)]
#[case::no_run_listing("GET", "/v1/runs", false)]
#[case::no_run_listing_with_slash("GET", "/v1/runs/", false)]
#[case::no_cancel_by_get("GET", "/v1/runs/run-1/cancel", false)]
#[case::no_cancel_extension("POST", "/v1/runs/run-1/cancel/extra", false)]
#[case::no_cancel_of_an_empty_id("POST", "/v1/runs//cancel", false)]
#[case::no_dot_segment_cancel("POST", "/v1/runs/../cancel", false)]
#[case::no_submit_by_get("GET", "/v1/automation/run", false)]
#[case::no_submit_tail("POST", "/v1/automation/run/extra", false)]
#[case::no_other_method("PUT", "/v1/automation/run", false)]
#[case::no_wallet_management("GET", "/v1/wallet/balance", false)]
#[case::no_vault_management("POST", "/v1/vault/items", false)]
#[case::no_leading_slash("POST", "v1/automation/run", false)]
#[case::no_root_path("/", "/", false)]
#[case::no_trailing_slash("POST", "/v1/automation/run/", false)]
#[case::no_double_slash("POST", "/v1//automation/run", false)]
#[case::no_dot_segment("POST", "/v1/automation/./run", false)]
#[case::no_dot_dot_segment("POST", "/v1/automation/../run", false)]
#[case::no_query_string("POST", "/v1/automation/run?stream=true", false)]
fn allows_only_the_run_operations(#[case] method: &str, #[case] path: &str, #[case] allowed: bool) {
    assert_eq!(is_allowed_tinyfish_endpoint(method, path), allowed);
}

#[test]
fn names_the_authenticated_run_fields_exactly() {
    assert_eq!(
        TINYFISH_RUN_AUTH_FIELDS,
        [
            "use_profile",
            "profile_id",
            "use_vault",
            "credential_item_ids"
        ]
    );
}

#[test]
fn names_the_rejected_envelope_fields_exactly() {
    assert_eq!(
        TINYFISH_ENVELOPE_FIELDS,
        ["custom_body", "stream", "query_params"]
    );
}

#[test]
fn gives_a_blocking_run_the_upstream_timeout_headroom() {
    assert_eq!(TINYFISH_TIMEOUT_SECONDS.to_bits(), 1500.0_f64.to_bits());
}

#[test]
fn names_the_poll_timings_and_terminal_statuses() {
    assert_eq!(TINYFISH_POLL_SECONDS, Duration::from_secs(5));
    assert_eq!(TINYFISH_MAX_POLL_SECONDS, Duration::from_secs(3600));
    assert_eq!(TINYFISH_MAX_POLL_FAILURES, 12);
    assert_eq!(
        TINYFISH_TERMINAL_STATUSES,
        ["COMPLETED", "FAILED", "CANCELLED"]
    );
}
