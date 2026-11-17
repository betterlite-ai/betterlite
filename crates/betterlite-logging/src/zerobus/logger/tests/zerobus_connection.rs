use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::zerobus::logger::connection::{ConnectionError, connection_for};
use crate::zerobus::logger::params::{ParamsError, configured_params};

use super::support::{MapSecrets, connection_secrets, params};

const ENDPOINT: &str = "https://1234567890123456.zerobus.us-west-2.cloud.databricks.com";
const WORKSPACE: &str = "https://dbc-1a2b3c4d.cloud.databricks.com";

fn entry(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn secrets_with_table(table: &str) -> MapSecrets {
    MapSecrets::new(&[
        ("ZEROBUS_SERVER_ENDPOINT", ENDPOINT),
        ("ZEROBUS_WORKSPACE_URL", WORKSPACE),
        ("ZEROBUS_CLIENT_ID", "sp-client-id"),
        ("ZEROBUS_CLIENT_SECRET", "sp-client-secret"),
        ("ZEROBUS_TABLE_NAME", table),
    ])
}

#[test]
fn zerobus_connection_reads_every_variable_from_the_environment() {
    let secrets = connection_secrets();
    let connection =
        connection_for(&params(100, 10, false), &secrets).expect("the connection resolves");
    assert_eq!(connection.server_endpoint, ENDPOINT);
    assert_eq!(connection.workspace_url, WORKSPACE);
    assert_eq!(connection.workspace_id, "1234567890123456");
    assert_eq!(connection.client_id, "sp-client-id");
    assert_eq!(connection.client_secret.expose_secret(), "sp-client-secret");
    assert_eq!(connection.table_name, "main.litellm.traces");
}

#[test]
fn zerobus_connection_prefers_the_configured_values() {
    let secrets = connection_secrets();
    let mut configured = params(100, 10, false);
    configured.server_endpoint =
        Some("https://999.zerobus.eu-central-1.cloud.databricks.com".to_owned());
    configured.table_name = Some("cfg.schema.table".to_owned());
    configured.client_secret = Some(SecretString::from("from-config"));
    let connection = connection_for(&configured, &secrets).expect("the configured values win");
    assert_eq!(
        connection.server_endpoint,
        "https://999.zerobus.eu-central-1.cloud.databricks.com"
    );
    assert_eq!(connection.workspace_id, "999");
    assert_eq!(connection.table_name, "cfg.schema.table");
    assert_eq!(connection.client_secret.expose_secret(), "from-config");
}

#[test]
fn zerobus_connection_resolves_an_environment_reference() {
    let secrets = MapSecrets::new(&[("SECRET_TOKEN", "resolved-secret")]);
    let mut configured = params(100, 10, false);
    configured.server_endpoint = Some(ENDPOINT.to_owned());
    configured.workspace_url = Some(WORKSPACE.to_owned());
    configured.client_id = Some("sp-client-id".to_owned());
    configured.client_secret = Some(SecretString::from("os.environ/SECRET_TOKEN"));
    configured.table_name = Some("main.litellm.traces".to_owned());
    let connection = connection_for(&configured, &secrets).expect("the reference resolves");
    assert_eq!(connection.client_secret.expose_secret(), "resolved-secret");
}

#[test]
fn zerobus_connection_keeps_a_plaintext_secret() {
    let secrets = MapSecrets::new(&[]);
    let mut configured = params(100, 10, false);
    configured.server_endpoint = Some(ENDPOINT.to_owned());
    configured.workspace_url = Some(WORKSPACE.to_owned());
    configured.client_id = Some("sp-client-id".to_owned());
    configured.client_secret = Some(SecretString::from("plain-secret"));
    configured.table_name = Some("main.litellm.traces".to_owned());
    let connection = connection_for(&configured, &secrets).expect("the plaintext secret stays");
    assert_eq!(connection.client_secret.expose_secret(), "plain-secret");
}

#[test]
fn zerobus_connection_falls_back_when_a_reference_does_not_resolve() {
    let secrets = secrets_with_table("main.litellm.traces");
    let mut configured = params(100, 10, false);
    configured.client_secret = Some(SecretString::from("os.environ/MISSING_TOKEN"));
    let connection = connection_for(&configured, &secrets).expect("the variable is the fallback");
    assert_eq!(connection.client_secret.expose_secret(), "sp-client-secret");
}

#[test]
fn zerobus_connection_names_the_missing_setting() {
    let secrets = MapSecrets::new(&[]);
    let error =
        connection_for(&params(100, 10, false), &secrets).expect_err("the endpoint is missing");
    let text = error.to_string();
    assert!(text.contains("ZEROBUS_SERVER_ENDPOINT"), "{text}");
    assert!(
        text.contains("callback_settings.zerobus.server_endpoint"),
        "{text}"
    );
}

#[test]
fn zerobus_connection_names_the_config_key_for_a_missing_secret() {
    let secrets = MapSecrets::new(&[
        ("ZEROBUS_SERVER_ENDPOINT", ENDPOINT),
        ("ZEROBUS_WORKSPACE_URL", WORKSPACE),
        ("ZEROBUS_CLIENT_ID", "sp-client-id"),
        ("ZEROBUS_TABLE_NAME", "main.litellm.traces"),
    ]);
    let error =
        connection_for(&params(100, 10, false), &secrets).expect_err("the secret is missing");
    let text = error.to_string();
    assert!(text.contains("ZEROBUS_CLIENT_SECRET"), "{text}");
    assert!(
        text.contains("callback_settings.zerobus.client_secret"),
        "{text}"
    );
}

#[test]
fn zerobus_connection_refuses_a_non_numeric_workspace_id() {
    let secrets = connection_secrets();
    let mut configured = params(100, 10, false);
    configured.server_endpoint = Some(WORKSPACE.to_owned());
    let error = connection_for(&configured, &secrets).expect_err("the label is not numeric");
    assert!(matches!(error, ConnectionError::WorkspaceId { .. }));
}

#[test]
fn zerobus_connection_refuses_a_malformed_endpoint() {
    let secrets = connection_secrets();
    let mut configured = params(100, 10, false);
    configured.server_endpoint = Some("not a url".to_owned());
    let error = connection_for(&configured, &secrets).expect_err("the endpoint cannot be parsed");
    assert!(matches!(error, ConnectionError::WorkspaceId { .. }));
}

#[test]
fn zerobus_connection_refuses_a_table_with_one_dot() {
    let secrets = secrets_with_table("main.traces");
    let error = connection_for(&params(100, 10, false), &secrets)
        .expect_err("the table is not fully qualified");
    let text = error.to_string();
    assert!(text.contains("catalog.schema.table"), "{text}");
    assert!(
        matches!(error, ConnectionError::TableName { table_name } if table_name == "main.traces")
    );
}

#[test]
fn zerobus_connection_refuses_a_table_with_three_dots() {
    let secrets = secrets_with_table("main.litellm.traces.extra");
    let error = connection_for(&params(100, 10, false), &secrets)
        .expect_err("the table has one part too many");
    assert!(matches!(error, ConnectionError::TableName { .. }));
}

#[test]
fn zerobus_connection_defaults_the_queue_settings() {
    let defaults = configured_params(None).expect("the defaults apply");
    assert_eq!(defaults.batch_size.get(), 100);
    assert_eq!(defaults.flush_interval.get(), 10);
    assert!(!defaults.turn_off_message_logging);
    assert!(defaults.server_endpoint.is_none());
    assert!(defaults.client_secret.is_none());
}

#[test]
fn zerobus_connection_reads_the_configured_settings() {
    let variables = entry(&[
        ("server_endpoint", ENDPOINT),
        ("workspace_url", WORKSPACE),
        ("client_id", "sp-client-id"),
        ("client_secret", "plain-secret"),
        ("table_name", "main.litellm.traces"),
        ("batch_size", "7"),
        ("flush_interval", "3"),
        ("turn_off_message_logging", "True"),
    ]);
    let configured = configured_params(Some(&variables)).expect("the settings parse");
    assert_eq!(configured.batch_size.get(), 7);
    assert_eq!(configured.flush_interval.get(), 3);
    assert!(configured.turn_off_message_logging);
    assert_eq!(
        configured.table_name.as_deref(),
        Some("main.litellm.traces")
    );
    assert_eq!(
        configured
            .client_secret
            .as_ref()
            .expect("the secret is set")
            .expose_secret(),
        "plain-secret"
    );
}

#[test]
fn zerobus_connection_uses_defaults_for_empty_settings() {
    let variables = entry(&[
        ("batch_size", ""),
        ("flush_interval", ""),
        ("turn_off_message_logging", ""),
    ]);
    let configured = configured_params(Some(&variables)).expect("empty settings fall back");
    assert_eq!(configured.batch_size.get(), 100);
    assert_eq!(configured.flush_interval.get(), 10);
    assert!(!configured.turn_off_message_logging);
}

#[test]
fn zerobus_connection_refuses_a_non_numeric_batch_size() {
    let worded = entry(&[("batch_size", "zero")]);
    let worded_error = configured_params(Some(&worded)).expect_err("not a number");
    assert!(matches!(worded_error, ParamsError::Number { field } if field == "batch_size"));
    let zeroed = entry(&[("batch_size", "0")]);
    let zeroed_error = configured_params(Some(&zeroed)).expect_err("zero is not a batch size");
    assert!(matches!(zeroed_error, ParamsError::Number { field } if field == "batch_size"));
}

#[test]
fn zerobus_connection_refuses_a_non_bool_flag() {
    let variables = entry(&[("turn_off_message_logging", "maybe")]);
    let error = configured_params(Some(&variables)).expect_err("not a bool");
    assert!(matches!(error, ParamsError::Bool { field } if field == "turn_off_message_logging"));
}

#[test]
fn zerobus_connection_refuses_an_unknown_setting() {
    let variables = entry(&[("bogus_setting", "1")]);
    let error = configured_params(Some(&variables)).expect_err("unknown key");
    assert!(matches!(error, ParamsError::Unknown { field } if field == "bogus_setting"));
}
