#![cfg(test)]

use rstest::rstest;

use super::Scenario;

#[rstest]
#[case::omitted(
    r#"{"model":"probe-model","custom_llm_provider":"openai"}"#,
    20.0_f64,
    0x0100_0000_usize,
    None
)]
#[case::empty(
    r#"{"model":"probe-model","custom_llm_provider":"openai","settings":{}}"#,
    20.0_f64,
    0x0100_0000_usize,
    None
)]
#[case::size_only(
    r#"{"model":"probe-model","custom_llm_provider":"openai","settings":{"max_message_size_bytes":2048}}"#,
    20.0_f64,
    2048_usize,
    None
)]
#[case::timeout_only(
    r#"{"model":"probe-model","custom_llm_provider":"openai","settings":{"credential_timeout_seconds":5.5}}"#,
    5.5_f64,
    0x0100_0000_usize,
    None
)]
#[case::protocol_only(
    r#"{"model":"probe-model","custom_llm_provider":"azure","settings":{"azure_realtime_protocol":"beta"}}"#,
    20.0_f64,
    0x0100_0000_usize,
    Some("beta")
)]
#[case::explicit_zero(
    r#"{"model":"probe-model","custom_llm_provider":"openai","settings":{"credential_timeout_seconds":0,"max_message_size_bytes":0}}"#,
    0.0_f64,
    0_usize,
    None
)]
fn probe_settings_defaults(
    #[case] source: &str,
    #[case] timeout: f64,
    #[case] size: usize,
    #[case] protocol: Option<&str>,
) {
    let scenario: Scenario = serde_json::from_str(source).unwrap();
    assert_eq!(
        scenario.settings.credential_timeout_seconds.to_bits(),
        timeout.to_bits()
    );
    assert_eq!(scenario.settings.max_message_size_bytes, size);
    assert_eq!(
        scenario.settings.azure_realtime_protocol.as_deref(),
        protocol
    );
}
