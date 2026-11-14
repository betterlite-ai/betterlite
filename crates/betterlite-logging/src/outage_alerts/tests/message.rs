use rstest::rstest;

use crate::outage_alerts::message::{
    OutageAlertType, OutageKey, OutageMessage, count_outage_alerts, outage_alert_message,
};

#[rstest]
#[case::timeout_408(&[408], "\nTimeout Errors: 1\n")]
#[case::server_error_500(&[500], "\nAPI Errors: 1\n")]
#[case::server_error_599(&[599], "\nAPI Errors: 1\n")]
#[case::client_error_499(&[499], "\nUnknown Errors: 1\n")]
#[case::client_error_401(&[401], "\nUnknown Errors: 1\n")]
#[case::client_error_429(&[429], "\nUnknown Errors: 1\n")]
#[case::no_alerts(&[], "")]
fn message_counts_each_status_in_its_bucket(#[case] alerts: &[i64], #[case] expected: &str) {
    assert_eq!(count_outage_alerts(alerts), expected);
}

#[test]
fn message_counts_buckets_in_timeout_api_unknown_order() {
    assert_eq!(
        count_outage_alerts(&[408, 500, 429, 500, 408]),
        "\nTimeout Errors: 2\n\nAPI Errors: 2\n\nUnknown Errors: 1\n"
    );
}

#[test]
fn message_writes_the_model_outage_with_the_api_base() {
    let message = OutageMessage {
        alert_type: OutageAlertType::Minor,
        key: OutageKey::Model,
        key_value: "gpt-4o",
        provider: "openai",
        api_base: Some("https://api.openai.com"),
        alerts: &[408, 500, 500, 429],
        now: 100.5,
        last_updated_at: 100.0,
    };
    assert_eq!(
        outage_alert_message(&message),
        "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Model Name:* `gpt-4o`\n*Provider:* `openai`\n*API Base:* `https://api.openai.com`\n\n\n*Errors:*\n\nTimeout Errors: 1\n\nAPI Errors: 2\n\nUnknown Errors: 1\n\n\n*Last Check:* `0.5s ago`\n\n\n"
    );
}

#[test]
fn message_writes_the_region_outage_without_the_api_base() {
    let message = OutageMessage {
        alert_type: OutageAlertType::Major,
        key: OutageKey::Region,
        key_value: "us-east1",
        provider: "vertex_ai",
        api_base: None,
        alerts: &[500, 502],
        now: 5.0,
        last_updated_at: 2.0,
    };
    assert_eq!(
        outage_alert_message(&message),
        "\n\n\n*\u{26a0}\u{fe0f} Major Service Outage*\n\n\n*Region Name:* `us-east1`\n*Provider:* `vertex_ai`\n\n\n*Errors:*\n\nAPI Errors: 2\n\n\n*Last Check:* `3.0s ago`\n\n\n"
    );
}

#[test]
fn message_leaves_the_errors_block_empty_without_counts() {
    let message = OutageMessage {
        alert_type: OutageAlertType::Minor,
        key: OutageKey::Model,
        key_value: "m",
        provider: "p",
        api_base: None,
        alerts: &[],
        now: 1.0,
        last_updated_at: 1.0,
    };
    assert_eq!(
        outage_alert_message(&message),
        "\n\n\n*\u{26a0}\u{fe0f} Minor Service Outage*\n\n\n*Model Name:* `m`\n*Provider:* `p`\n\n\n*Errors:*\n\n\n*Last Check:* `0.0s ago`\n\n\n"
    );
}

#[rstest]
#[case::half_a_second(
    100.5,
    100.0,
    "\n\n\n*\u{26a0}\u{fe0f} Major Service Outage*\n\n\n*Region Name:* `r`\n*Provider:* `p`\n\n\n*Errors:*\n\nAPI Errors: 1\n\n\n*Last Check:* `0.5s ago`\n\n\n"
)]
#[case::whole_seconds_keep_a_decimal(
    5.0,
    2.0,
    "\n\n\n*\u{26a0}\u{fe0f} Major Service Outage*\n\n\n*Region Name:* `r`\n*Provider:* `p`\n\n\n*Errors:*\n\nAPI Errors: 1\n\n\n*Last Check:* `3.0s ago`\n\n\n"
)]
#[case::four_decimal_places(
    0.0002,
    0.0001,
    "\n\n\n*\u{26a0}\u{fe0f} Major Service Outage*\n\n\n*Region Name:* `r`\n*Provider:* `p`\n\n\n*Errors:*\n\nAPI Errors: 1\n\n\n*Last Check:* `0.0001s ago`\n\n\n"
)]
#[case::ties_round_to_even(
    10.03125,
    10.0,
    "\n\n\n*\u{26a0}\u{fe0f} Major Service Outage*\n\n\n*Region Name:* `r`\n*Provider:* `p`\n\n\n*Errors:*\n\nAPI Errors: 1\n\n\n*Last Check:* `0.0312s ago`\n\n\n"
)]
#[case::no_elapsed_time(
    1.0,
    1.0,
    "\n\n\n*\u{26a0}\u{fe0f} Major Service Outage*\n\n\n*Region Name:* `r`\n*Provider:* `p`\n\n\n*Errors:*\n\nAPI Errors: 1\n\n\n*Last Check:* `0.0s ago`\n\n\n"
)]
fn message_rounds_the_last_check_to_four_decimals(
    #[case] now: f64,
    #[case] last_updated_at: f64,
    #[case] expected: &str,
) {
    let message = OutageMessage {
        alert_type: OutageAlertType::Major,
        key: OutageKey::Region,
        key_value: "r",
        provider: "p",
        api_base: None,
        alerts: &[500],
        now,
        last_updated_at,
    };
    assert_eq!(outage_alert_message(&message), expected);
}
