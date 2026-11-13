use betterlite_logging::zerobus::client::init_params::ZerobusInitParams;
use rstest::rstest;
use secrecy::ExposeSecret as _;

#[test]
fn the_defaults_are_batch_size_one_hundred_and_flush_interval_ten() {
    let params = ZerobusInitParams::default();
    assert_eq!(params.batch_size.get(), 100);
    assert_eq!(params.flush_interval.get(), 10);
    assert!(!params.turn_off_message_logging);
    assert!(params.workspace_url.is_none());
    assert!(params.server_endpoint.is_none());
    assert!(params.client_id.is_none());
    assert!(params.client_secret.is_none());
    assert!(params.table_name.is_none());
}

#[test]
fn an_empty_configuration_takes_the_same_defaults() {
    let params: ZerobusInitParams = serde_json::from_str("{}").unwrap();
    assert_eq!(params.batch_size.get(), 100);
    assert_eq!(params.flush_interval.get(), 10);
    assert!(!params.turn_off_message_logging);
}

#[test]
fn a_null_message_logging_mode_takes_the_default() {
    let params: ZerobusInitParams =
        serde_json::from_str(r#"{"turn_off_message_logging": null}"#).unwrap();
    assert!(!params.turn_off_message_logging);
}

#[rstest]
#[case::zero_batch_size(r#"{"batch_size": 0}"#)]
#[case::negative_batch_size(r#"{"batch_size": -1}"#)]
#[case::zero_flush_interval(r#"{"flush_interval": 0}"#)]
#[case::negative_flush_interval(r#"{"flush_interval": -1}"#)]
fn a_nonpositive_limit_is_rejected(#[case] text: &str) {
    serde_json::from_str::<ZerobusInitParams>(text).unwrap_err();
}

#[test]
fn one_is_the_smallest_accepted_limit() {
    let params: ZerobusInitParams =
        serde_json::from_str(r#"{"batch_size": 1, "flush_interval": 1}"#).unwrap();
    assert_eq!(params.batch_size.get(), 1);
    assert_eq!(params.flush_interval.get(), 1);
}

#[test]
fn the_connection_fields_and_message_logging_mode_parse() {
    let params: ZerobusInitParams = serde_json::from_str(
        r#"{
            "workspace_url": "https://dbc.example.cloud.databricks.com",
            "server_endpoint": "https://1234.zerobus.us-west-2.cloud.databricks.com",
            "client_id": "sp-client-id",
            "client_secret": "sp-client-secret",
            "table_name": "main.litellm.traces",
            "batch_size": 7,
            "flush_interval": 3,
            "turn_off_message_logging": true
        }"#,
    )
    .unwrap();
    assert_eq!(
        params.workspace_url.as_deref(),
        Some("https://dbc.example.cloud.databricks.com")
    );
    assert_eq!(
        params.server_endpoint.as_deref(),
        Some("https://1234.zerobus.us-west-2.cloud.databricks.com")
    );
    assert_eq!(params.client_id.as_deref(), Some("sp-client-id"));
    assert_eq!(
        params.client_secret.as_ref().unwrap().expose_secret(),
        "sp-client-secret"
    );
    assert_eq!(params.table_name.as_deref(), Some("main.litellm.traces"));
    assert_eq!(params.batch_size.get(), 7);
    assert_eq!(params.flush_interval.get(), 3);
    assert!(params.turn_off_message_logging);
}

#[test]
fn the_client_secret_stays_out_of_the_debug_output() {
    let params: ZerobusInitParams =
        serde_json::from_str(r#"{"client_secret": "sp-client-secret"}"#).unwrap();
    assert!(!format!("{params:?}").contains("sp-client-secret"));
}
