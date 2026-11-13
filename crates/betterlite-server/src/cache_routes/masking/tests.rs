#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_redact::structured::SensitiveDataMasker;
use betterlite_redis::redis_connection_settings::model::scalars::FloatOrText;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use serde_json::Number;

use super::super::redis_support::overrides;
use super::redis_kwargs;

fn parsed(text: &str) -> RedisOverrides {
    serde_json::from_str(text).expect("the overrides deserialize")
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn masked(value: &str) -> Json {
    text(&SensitiveDataMasker::default().mask_value(value))
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).unwrap())
}

#[test]
fn nothing_but_the_default_socket_timeout_is_shown_for_empty_overrides() {
    let kwargs = redis_kwargs(&overrides());
    assert_eq!(kwargs.len(), 1);
    assert_eq!(kwargs.get("socket_timeout"), Some(&float(5.0_f64)));
}

#[test]
fn an_explicit_socket_timeout_replaces_the_default() {
    let mut settings = overrides();
    settings.socket_timeout = Some(FloatOrText::Value(1.5_f64));
    let kwargs = redis_kwargs(&settings);
    assert_eq!(kwargs.get("socket_timeout"), Some(&float(1.5_f64)));
}

#[test]
fn the_host_port_and_connection_options_appear_as_given() {
    let mut settings = parsed(
        r#"{"host": "redis.internal", "port": "6380", "db": 2, "username": "proxy", "ssl": true,
            "socket_keepalive": false, "max_connections": 20, "health_check_interval": 30}"#,
    );
    settings.socket_connect_timeout = Some(FloatOrText::Value(2.5_f64));
    let kwargs = redis_kwargs(&settings);
    assert_eq!(kwargs.get("host"), Some(&text("redis.internal")));
    assert_eq!(kwargs.get("port"), Some(&text("6380")));
    assert_eq!(kwargs.get("db"), Some(&number(2_i64)));
    assert_eq!(kwargs.get("username"), Some(&text("proxy")));
    assert_eq!(kwargs.get("ssl"), Some(&Json::Bool(true)));
    assert_eq!(kwargs.get("socket_connect_timeout"), Some(&float(2.5_f64)));
    assert_eq!(kwargs.get("socket_keepalive"), Some(&Json::Bool(false)));
    assert_eq!(kwargs.get("max_connections"), Some(&number(20_i64)));
    assert_eq!(kwargs.get("health_check_interval"), Some(&number(30_i64)));
}

#[test]
fn the_password_is_masked() {
    let kwargs = redis_kwargs(&parsed(r#"{"password": "hunter2-and-more-text"}"#));
    assert_eq!(
        kwargs.get("password"),
        Some(&masked("hunter2-and-more-text"))
    );
}

#[test]
fn a_null_password_is_left_out() {
    let kwargs = redis_kwargs(&parsed(r#"{"password": null}"#));
    assert_eq!(kwargs.get("password"), None);
}

#[test]
fn every_secret_by_segment_name_is_masked_and_other_keys_are_not() {
    let kwargs = redis_kwargs(&parsed(
        r#"{"sentinel_password": "sentinel-secret-value", "ssl_password": "ssl-secret-value-here",
            "azure_redis_ad_token": "azure-token-value-1", "azure_client_secret": "azure-client-secret-1",
            "ssl_keyfile": "/etc/ssl/private/redis.key", "service_name": "mymaster"}"#,
    ));
    assert_eq!(
        kwargs.get("sentinel_password"),
        Some(&masked("sentinel-secret-value"))
    );
    assert_eq!(
        kwargs.get("ssl_password"),
        Some(&masked("ssl-secret-value-here"))
    );
    assert_eq!(
        kwargs.get("azure_redis_ad_token"),
        Some(&masked("azure-token-value-1"))
    );
    assert_eq!(
        kwargs.get("azure_client_secret"),
        Some(&masked("azure-client-secret-1"))
    );
    assert_eq!(
        kwargs.get("ssl_keyfile"),
        Some(&text("/etc/ssl/private/redis.key"))
    );
    assert_eq!(kwargs.get("service_name"), Some(&text("mymaster")));
}

#[test]
fn the_cloud_identity_settings_appear_with_sensitive_names_masked() {
    let kwargs = redis_kwargs(&parsed(
        r#"{"gcp_service_account": "sa@project", "gcp_ssl_ca_certs": "/ca.pem", "azure_client_id": "client",
            "azure_tenant_id": "tenant", "aws_iam_auth": true, "aws_iam_user_name": "user",
            "aws_iam_cache_name": "cache", "aws_iam_region": "eu-west-1", "aws_iam_serverless": false}"#,
    ));
    assert_eq!(kwargs.get("gcp_service_account"), Some(&text("sa@project")));
    assert_eq!(kwargs.get("gcp_ssl_ca_certs"), Some(&text("/ca.pem")));
    assert_eq!(kwargs.get("azure_client_id"), Some(&text("client")));
    assert_eq!(kwargs.get("azure_tenant_id"), Some(&text("tenant")));
    assert_eq!(kwargs.get("aws_iam_auth"), Some(&masked("true")));
    assert_eq!(kwargs.get("aws_iam_user_name"), Some(&text("user")));
    assert_eq!(kwargs.get("aws_iam_cache_name"), Some(&text("cache")));
    assert_eq!(kwargs.get("aws_iam_region"), Some(&text("eu-west-1")));
    assert_eq!(kwargs.get("aws_iam_serverless"), Some(&Json::Bool(false)));
}

#[test]
fn the_startup_nodes_are_listed() {
    let kwargs = redis_kwargs(&parsed(
        r#"{"startup_nodes": [{"host": "10.0.0.1", "port": "7000"}, {"host": "10.0.0.2", "port": 7001}]}"#,
    ));
    let Some(Json::List(nodes)) = kwargs.get("startup_nodes") else {
        panic!("the startup nodes are a list");
    };
    assert_eq!(nodes.len(), 2);
    let Some(Json::Object(first)) = nodes.first() else {
        panic!("a node is an object");
    };
    assert_eq!(first.get("host"), Some(&text("10.0.0.1")));
    assert_eq!(first.get("port"), Some(&text("7000")));
    let Some(Json::Object(second)) = nodes.get(1) else {
        panic!("a node is an object");
    };
    assert_eq!(second.get("port"), Some(&number(7001_i64)));
}

#[test]
fn the_sentinel_nodes_are_listed() {
    let kwargs = redis_kwargs(&parsed(r#"{"sentinel_nodes": [["10.0.0.9", 26379]]}"#));
    assert_eq!(
        kwargs.get("sentinel_nodes"),
        Some(&Json::List(vec![Json::List(vec![
            text("10.0.0.9"),
            number(26379_i64),
        ])]))
    );
}

#[test]
fn a_node_list_given_as_text_is_shown_as_text() {
    let kwargs = redis_kwargs(&parsed(r#"{"startup_nodes": "os.environ/NODES"}"#));
    assert_eq!(kwargs.get("startup_nodes"), Some(&text("os.environ/NODES")));
}

#[test]
fn the_passthrough_keys_are_listed_and_sensitive_ones_masked() {
    let kwargs = redis_kwargs(&parsed(
        r#"{"ssl_cert_reqs": "none", "retry_on_timeout": true, "auth_token": "passthrough-token-1",
            "socket_keepalive_options": {"1": 3}, "retry_on_error": ["timeout", 5]}"#,
    ));
    assert_eq!(kwargs.get("ssl_cert_reqs"), Some(&text("none")));
    assert_eq!(kwargs.get("retry_on_timeout"), Some(&Json::Bool(true)));
    assert_eq!(
        kwargs.get("auth_token"),
        Some(&masked("passthrough-token-1"))
    );
    let Some(Json::Object(options)) = kwargs.get("socket_keepalive_options") else {
        panic!("the keepalive options are an object");
    };
    assert_eq!(options.get("1"), Some(&number(3_i64)));
    assert_eq!(
        kwargs.get("retry_on_error"),
        Some(&Json::List(vec![text("timeout"), number(5_i64)]))
    );
}

#[test]
fn a_sensitive_number_and_flag_are_masked_and_a_list_masks_only_its_text() {
    let kwargs = redis_kwargs(&parsed(
        r#"{"secret_number": 123456789, "secret_flag": true, "secret_list": ["first-secret-value", 4]}"#,
    ));
    assert_eq!(kwargs.get("secret_number"), Some(&masked("123456789")));
    assert_eq!(kwargs.get("secret_flag"), Some(&masked("true")));
    assert_eq!(
        kwargs.get("secret_list"),
        Some(&Json::List(vec![
            masked("first-secret-value"),
            number(4_i64)
        ]))
    );
}

#[test]
fn the_url_password_and_sensitive_query_values_are_masked() {
    let kwargs = redis_kwargs(&parsed(
        r#"{"url": "redis://proxy:hunter2-and-more-text@redis.internal:6379/0?ssl_cert_reqs=none&password=query-secret-value"}"#,
    ));
    let masker = SensitiveDataMasker::default();
    let expected = format!(
        "redis://proxy:{}@redis.internal:6379/0?ssl_cert_reqs=none&password={}",
        masker.mask_value("hunter2-and-more-text"),
        masker.mask_value("query-secret-value"),
    );
    assert_eq!(kwargs.get("url"), Some(&text(&expected)));
}

#[test]
fn a_url_without_credentials_is_shown_whole() {
    let kwargs = redis_kwargs(&parsed(r#"{"url": "redis://redis.internal:6379/0"}"#));
    assert_eq!(
        kwargs.get("url"),
        Some(&text("redis://redis.internal:6379/0"))
    );
}

#[test]
fn a_url_that_does_not_parse_is_masked_whole() {
    let kwargs = redis_kwargs(&parsed(r#"{"url": "not a url but with a password in it"}"#));
    assert_eq!(
        kwargs.get("url"),
        Some(&masked("not a url but with a password in it"))
    );
}

#[test]
fn a_null_url_is_left_out() {
    let kwargs = redis_kwargs(&parsed(r#"{"url": null}"#));
    assert_eq!(kwargs.get("url"), None);
}
