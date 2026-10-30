#![cfg(test)]

use rstest::rstest;

use crate::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

#[rstest]
#[case::unix_system_path(
    "failed to read /etc/betterlite/secrets.yaml",
    "failed to read REDACTED"
)]
#[case::windows_drive_path(r"failed to read C:\Users\svc\secrets.yaml", "failed to read REDACTED")]
#[case::private_ipv4("connect refused from 10.0.0.5", "connect refused from REDACTED")]
#[case::loopback_ipv4("bound to 127.0.0.1", "bound to REDACTED")]
#[case::internal_suffix_hostname("upstream api.gateway.internal down", "upstream REDACTED down")]
#[case::public_ip_not_internal("client at 8.8.8.8 retried", "client at 8.8.8.8 retried")]
#[case::traceback_dropped(
    "request failed\nTraceback (most recent call last):\n  File \"/etc/app.py\", line 1",
    "request failed"
)]
#[case::secret_scrubbed_before_internal_detail(
    "api_key='sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' at /etc/app/config",
    "REDACTED' at REDACTED"
)]
#[case::combining_mark_before_private_ipv4_is_redacted(
    "\u{301}10.0.0.5 is down",
    "\u{301}REDACTED is down"
)]
#[case::combining_mark_before_internal_suffix_hostname_is_redacted(
    "\u{301}api.gateway.internal is down",
    "\u{301}REDACTED is down"
)]
#[case::dotless_i_inside_an_internal_hostname_is_redacted(
    "see db\u{131}.internal now",
    "see REDACTED now"
)]
fn secret_redaction_redaction_tests_behavior(#[case] input: &str, #[case] expected: &str) {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    assert_eq!(redactor.redact_internal_details(input), expected);
}
