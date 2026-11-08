use std::collections::BTreeMap;

use redis::cmd;
use tempfile::NamedTempFile;

use super::fake_server::{FakeServer, mutual_tls, server_config, temp_pem, test_certificate};
use super::fixtures::{Logs, direct_fields, factory, open_error};
use crate::redis_client_factories::RedisClient;
use crate::redis_client_factories::fields::ConnectionFields;
use crate::redis_connection_settings::model::scalars::PassthroughValue;
use crate::redis_live_client::{LiveClientError, LiveTarget};

fn with_passthrough(entries: &[(&str, PassthroughValue)]) -> BTreeMap<String, PassthroughValue> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[tokio::test]
async fn a_ca_certificate_authenticates_the_server() {
    let certificate = test_certificate(&["localhost"]);
    let server = FakeServer::start_tls(server_config(&certificate, None)).await;
    let ca = temp_pem(&certificate.cert_pem);
    let mut fields = direct_fields("localhost", server.port());
    fields.ssl = Some(true);
    fields.socket_timeout = Some(0.5_f64);
    fields.passthrough = with_passthrough(&[(
        "ssl_ca_certs",
        PassthroughValue::Text(ca.path().to_str().expect("a UTF-8 path").to_owned()),
    )]);
    let client = factory(None)
        .open_async(&RedisClient::Direct(fields), None)
        .await
        .expect("a TLS connection");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
    assert_eq!(server.tls_connections(), 1);
}

#[tokio::test]
async fn an_untrusted_certificate_fails() {
    let certificate = test_certificate(&["localhost"]);
    let server = FakeServer::start_tls(server_config(&certificate, None)).await;
    let other = test_certificate(&["localhost"]);
    let ca = temp_pem(&other.cert_pem);
    let mut fields = direct_fields("localhost", server.port());
    fields.ssl = Some(true);
    fields.socket_timeout = Some(0.5_f64);
    fields.passthrough = with_passthrough(&[(
        "ssl_ca_certs",
        PassthroughValue::Text(ca.path().to_str().expect("a UTF-8 path").to_owned()),
    )]);
    let error = open_error(&factory(None), &RedisClient::Direct(fields)).await;
    assert!(matches!(error, LiveClientError::Redis(_)));
    let text = error.to_string().to_lowercase();
    assert!(
        text.contains("unknown issuer") || text.contains("unknownissuer"),
        "the handshake should fail on the issuer: {text}"
    );
}

#[tokio::test]
async fn ssl_cert_reqs_none_skips_verification() {
    let certificate = test_certificate(&["localhost"]);
    let server = FakeServer::start_tls(server_config(&certificate, None)).await;
    let mut fields = direct_fields("localhost", server.port());
    fields.ssl = Some(true);
    fields.socket_timeout = Some(0.5_f64);
    fields.passthrough =
        with_passthrough(&[("ssl_cert_reqs", PassthroughValue::Text("none".to_owned()))]);
    let client = factory(None)
        .open_async(&RedisClient::Direct(fields), None)
        .await
        .expect("an insecure TLS connection");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
}

fn other_name_fields(
    server: &FakeServer,
    ca: &NamedTempFile,
    check_hostname: Option<bool>,
) -> ConnectionFields {
    let mut fields = direct_fields("localhost", server.port());
    fields.ssl = Some(true);
    fields.socket_timeout = Some(0.5_f64);
    let mut passthrough = with_passthrough(&[(
        "ssl_ca_certs",
        PassthroughValue::Text(ca.path().to_str().expect("a UTF-8 path").to_owned()),
    )]);
    if let Some(check) = check_hostname {
        passthrough.insert(
            "ssl_check_hostname".to_owned(),
            PassthroughValue::Bool(check),
        );
    }
    fields.passthrough = passthrough;
    fields
}

#[tokio::test]
async fn a_certificate_for_another_name_is_rejected_by_the_hostname_check() {
    let certificate = test_certificate(&["other.example"]);
    let server = FakeServer::start_tls(server_config(&certificate, None)).await;
    let ca = temp_pem(&certificate.cert_pem);
    let fields = other_name_fields(&server, &ca, None);
    let error = open_error(&factory(None), &RedisClient::Direct(fields)).await;
    let text = error.to_string().to_lowercase();
    assert!(
        text.contains("not valid for name") || text.contains("notvalidforname"),
        "the handshake should fail on the certificate name: {text}"
    );
}

#[tokio::test]
async fn ssl_check_hostname_false_accepts_another_name() {
    let certificate = test_certificate(&["other.example"]);
    let server = FakeServer::start_tls(server_config(&certificate, None)).await;
    let ca = temp_pem(&certificate.cert_pem);
    let fields = other_name_fields(&server, &ca, Some(false));
    let client = factory(None)
        .open_async(&RedisClient::Direct(fields), None)
        .await
        .expect("a connection that skips the hostname check");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
}

#[tokio::test]
async fn the_tls_keys_work_in_a_rediss_url_query() {
    let certificate = test_certificate(&["localhost"]);
    let server = FakeServer::start_tls(server_config(&certificate, None)).await;
    let ca = temp_pem(&certificate.cert_pem);
    let mut url =
        url::Url::parse(&format!("rediss://localhost:{}/0", server.port())).expect("a redis url");
    url.query_pairs_mut()
        .append_pair("ssl_ca_certs", ca.path().to_str().expect("a UTF-8 path"));
    let record = super::fixtures::url_client(url.to_string());
    let client = factory(None)
        .open_async(&RedisClient::Url(record), None)
        .await
        .expect("a TLS URL connection");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
}

#[tokio::test]
async fn a_client_certificate_reaches_the_server() {
    let certificate = test_certificate(&["localhost"]);
    let mutual = mutual_tls();
    let server = FakeServer::start_tls(server_config(&certificate, Some(&mutual.ca))).await;
    let ca = temp_pem(&certificate.cert_pem);
    let client_certificate = temp_pem(&mutual.cert_pem);
    let client_key = temp_pem(&mutual.key_pem);
    let mut fields = direct_fields("localhost", server.port());
    fields.ssl = Some(true);
    fields.socket_timeout = Some(0.5_f64);
    fields.passthrough = with_passthrough(&[
        (
            "ssl_ca_certs",
            PassthroughValue::Text(ca.path().to_str().expect("a UTF-8 path").to_owned()),
        ),
        (
            "ssl_certfile",
            PassthroughValue::Text(
                client_certificate
                    .path()
                    .to_str()
                    .expect("a UTF-8 path")
                    .to_owned(),
            ),
        ),
        (
            "ssl_keyfile",
            PassthroughValue::Text(client_key.path().to_str().expect("a UTF-8 path").to_owned()),
        ),
    ]);
    let client = factory(None)
        .open_async(&RedisClient::Direct(fields), None)
        .await
        .expect("a mutual TLS connection");
    let reply: String = client.query(&cmd("PING")).await.expect("a reply");
    assert_eq!(reply, "PONG");
    assert!(server.admitted_client_certificate());
}

#[tokio::test]
async fn a_missing_tls_file_is_a_tls_file_error() {
    let mut fields = direct_fields("127.0.0.1", 6379);
    fields.ssl = Some(true);
    fields.passthrough = with_passthrough(&[(
        "ssl_ca_certs",
        PassthroughValue::Text("/definitely/missing/ca.pem".to_owned()),
    )]);
    let error = open_error(&factory(None), &RedisClient::Direct(fields)).await;
    assert!(matches!(error, LiveClientError::TlsFile { key } if key == "ssl_ca_certs"));
}

#[test]
fn an_unmapped_passthrough_key_logs_its_name_once() {
    let mut fields = direct_fields("127.0.0.1", 6379);
    fields.passthrough = with_passthrough(&[(
        "retry_on_error",
        PassthroughValue::Text("topsecret-value".to_owned()),
    )]);
    let (logs, _guard) = Logs::capture();
    let endpoint = factory(None)
        .blocking_endpoint(&RedisClient::Direct(fields))
        .expect("an endpoint");
    assert!(matches!(endpoint.target, LiveTarget::Standalone(_)));
    let warnings = logs.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("key=retry_on_error"), "{warnings:?}");
    assert!(!warnings[0].contains("topsecret-value"));
}
