use std::collections::BTreeMap;

use secrecy::SecretString;
use tracing::subscriber::with_default;

use super::fake_server::{FakeServer, Reply};
use super::fixtures::{
    Capture, assert_no_secret, closed_address, direct_fields, endpoint_error, factory, open_error,
    sentinel_record, url_client,
};
use crate::redis_client_factories::RedisClient;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_live_client::LiveClientError;

#[tokio::test]
async fn parse_failures_do_not_expose_the_url_password() {
    let record = url_client("redis://user:topsecret@127.0.0.1:1/not-a-number".to_owned());
    let error = open_error(&factory(None), &RedisClient::Url(record)).await;
    assert_no_secret(&error, "topsecret");
}

#[tokio::test]
async fn connect_failures_do_not_expose_the_password() {
    let mut fields = direct_fields("127.0.0.1", closed_address().port());
    fields.password = Some(SecretString::from("topsecret".to_owned()));
    fields.socket_timeout = Some(0.2_f64);
    let error = open_error(&factory(None), &RedisClient::Direct(fields)).await;
    assert_no_secret(&error, "topsecret");
}

#[tokio::test]
async fn auth_failures_do_not_expose_the_token() {
    let server = FakeServer::start().await;
    server.script(
        "AUTH",
        Reply::Error("WRONGPASS invalid username-password pair or user is disabled.".to_owned()),
    );
    let mut fields = direct_fields(&server.host(), server.port());
    fields.password = Some(SecretString::from("topsecret".to_owned()));
    fields.socket_timeout = Some(0.5_f64);
    let error = open_error(&factory(None), &RedisClient::Direct(fields)).await;
    assert_no_secret(&error, "topsecret");
}

#[tokio::test]
async fn tls_failures_do_not_expose_passthrough_values() {
    let mut fields = direct_fields("127.0.0.1", 6379);
    fields.ssl = Some(true);
    fields.passthrough = BTreeMap::from([(
        "ssl_ca_certs".to_owned(),
        PassthroughValue::Text("/missing/topsecret-ca.pem".to_owned()),
    )]);
    let error = open_error(&factory(None), &RedisClient::Direct(fields)).await;
    assert!(matches!(error, LiveClientError::TlsFile { key } if key == "ssl_ca_certs"));
    assert_no_secret(&error, "topsecret");
}

#[test]
fn log_lines_do_not_expose_the_monitor_password() {
    let mut record = sentinel_record(&[closed_address()], "mymaster");
    record.monitor_password = Some(SecretString::from("topsecret".to_owned()));
    let capture = Capture::default();
    let error = with_default(capture.clone(), || {
        endpoint_error(&factory(None), &RedisClient::Sentinel(record))
    });
    assert!(matches!(error, LiveClientError::NoSentinelMaster { .. }));
    let leaking: Vec<String> = capture
        .warnings()
        .into_iter()
        .map(|warning| warning.text())
        .filter(|text| text.contains("topsecret"))
        .collect();
    assert!(
        leaking.is_empty(),
        "a log line carries the monitor password: {leaking:?}"
    );
}

#[test]
fn log_lines_do_not_expose_a_passthrough_value() {
    let mut fields = direct_fields("127.0.0.1", 6379);
    fields.passthrough = BTreeMap::from([(
        "retry_on_error".to_owned(),
        PassthroughValue::Text("topsecret-value".to_owned()),
    )]);
    let capture = Capture::default();
    let _endpoint = with_default(capture.clone(), || {
        factory(None)
            .blocking_endpoint(&RedisClient::Direct(fields))
            .expect("an endpoint")
    });
    let warnings = capture.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].field("key"), Some("retry_on_error"));
    assert!(!warnings[0].text().contains("topsecret-value"));
}
