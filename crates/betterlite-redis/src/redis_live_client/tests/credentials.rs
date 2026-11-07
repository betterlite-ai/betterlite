use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::time::sleep;

use redis::Value;
use redis::cmd;

use super::fake_server::FakeServer;
use super::fixtures::{
    FailingSource, TestSource, assert_no_secret, cluster_pair, cluster_record, credential,
    direct_fields, endpoint_error, factory, key_in_slot_range, open_error, sentinel_record, source,
    url_client,
};
use crate::redis_client_factories::RedisClient;
use crate::redis_client_factories::fields::ConnectionFields;
use crate::redis_connection_settings::model::settings::CredentialProviderSettings;
use crate::redis_live_client::LiveClientError;
use crate::redis_live_client::credentials::RedisCredentialSource;

fn provider_fields(server: &FakeServer) -> ConnectionFields {
    let mut fields = direct_fields(&server.host(), server.port());
    fields.credential_provider = true;
    fields
}

#[tokio::test]
async fn an_injected_token_is_the_auth_sent_at_connect() {
    let server = FakeServer::start().await;
    let credential_source = source(vec![credential(Some("app"), "tok-1")]);
    let client = factory(Some(credential_source))
        .open_async(&RedisClient::Direct(provider_fields(&server)), None)
        .await
        .expect("a direct connection");
    client.query::<String>(&cmd("PING")).await.expect("a reply");
    let auth = server.commands_named("AUTH");
    assert_eq!(auth[0].args, ["AUTH", "app", "tok-1"]);
}

#[tokio::test]
async fn a_source_without_a_username_sends_default() {
    let server = FakeServer::start().await;
    let credential_source = source(vec![credential(None, "tok-1")]);
    let client = factory(Some(credential_source))
        .open_async(&RedisClient::Direct(provider_fields(&server)), None)
        .await
        .expect("a direct connection");
    client.query::<String>(&cmd("PING")).await.expect("a reply");
    let auth = server.commands_named("AUTH");
    assert_eq!(auth[0].args, ["AUTH", "default", "tok-1"]);
}

#[tokio::test]
async fn a_url_userinfo_is_dropped_when_a_source_is_active() {
    let server = FakeServer::start().await;
    let url = format!(
        "redis://url-user:url-password@{}:{}/0",
        server.host(),
        server.port()
    );
    let mut record = url_client(url);
    record.credential_provider = true;
    let credential_source = source(vec![credential(Some("token-user"), "token-value")]);
    let client = factory(Some(credential_source))
        .open_async(&RedisClient::Url(record), None)
        .await
        .expect("a url connection");
    client.query::<String>(&cmd("PING")).await.expect("a reply");
    let auth = server.commands_named("AUTH");
    assert_eq!(auth[0].args, ["AUTH", "token-user", "token-value"]);
    assert!(
        !auth
            .iter()
            .any(|recorded| recorded.args.iter().any(|arg| arg == "url-password")),
        "the URL password never authenticates"
    );
}

#[tokio::test]
async fn a_new_token_is_sent_after_the_refresh_interval() {
    let server = FakeServer::start().await;
    let rotating = TestSource::new(vec![
        credential(None, "tok-1"),
        credential(None, "tok-1"),
        credential(None, "tok-2"),
    ])
    .refreshing(Duration::from_millis(50));
    let credential_source: Arc<dyn RedisCredentialSource> = Arc::new(rotating);
    let client = factory(Some(credential_source))
        .open_async(&RedisClient::Direct(provider_fields(&server)), None)
        .await
        .expect("a direct connection");
    client.query::<String>(&cmd("PING")).await.expect("a reply");
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(2))
        .expect("a deadline");
    let mut auth = server.commands_named("AUTH");
    while auth.len() < 2 && Instant::now() < deadline {
        sleep(Duration::from_millis(10)).await;
        auth = server.commands_named("AUTH");
    }
    assert!(
        auth.iter()
            .any(|recorded| recorded.args == ["AUTH", "default", "tok-2"]),
        "the refreshed token re-authenticates the open connection: {auth:?}"
    );
}

#[tokio::test]
async fn a_wrong_argument_count_falls_back_to_the_token_alone() {
    let server = FakeServer::start().await;
    server.reject_username_auth();
    let credential_source = source(vec![credential(Some("app"), "tok-1")]);
    let client = factory(Some(credential_source))
        .open_async(&RedisClient::Direct(provider_fields(&server)), None)
        .await
        .expect("a direct connection");
    client.query::<String>(&cmd("PING")).await.expect("a reply");
    let auth = server.commands_named("AUTH");
    assert!(
        auth.iter()
            .any(|recorded| recorded.args == ["AUTH", "app", "tok-1"]),
        "the username form is tried first"
    );
    assert!(
        auth.iter()
            .any(|recorded| recorded.args == ["AUTH", "tok-1"]),
        "the username-less form follows: {auth:?}"
    );
}

#[tokio::test]
async fn cluster_data_nodes_authenticate_with_the_token() {
    let (first, second) = cluster_pair().await;
    let mut record = cluster_record(&[&first]);
    record.credential_provider = true;
    let credential_source = source(vec![credential(None, "cluster-token")]);
    let client = factory(Some(credential_source))
        .open_async(&RedisClient::Cluster(record), None)
        .await
        .expect("a cluster connection");
    let key = key_in_slot_range(8192, 16383);
    client
        .query::<Value>(cmd("SET").arg(key).arg("1"))
        .await
        .expect("a write to the second node");
    let first_auth = first.commands_named("AUTH");
    assert!(
        first_auth
            .iter()
            .any(|recorded| recorded.args == ["AUTH", "default", "cluster-token"]),
        "the first node authenticates with the injected token"
    );
    let second_auth = second.commands_named("AUTH");
    assert!(
        second_auth
            .iter()
            .any(|recorded| recorded.args == ["AUTH", "default", "cluster-token"]),
        "the second node authenticates with the injected token"
    );
}

#[tokio::test]
async fn sentinel_data_nodes_authenticate_with_the_token() {
    let master = FakeServer::start().await;
    let monitor = FakeServer::start().await;
    monitor.set_sentinel_master(master.host(), master.port());
    let mut record = sentinel_record(&[monitor.addr()], "mymaster");
    record.connection_kwargs.credential_provider = true;
    let credential_source = source(vec![credential(Some("data-user"), "data-token")]);
    let client = factory(Some(credential_source))
        .open_async(&RedisClient::Sentinel(record), None)
        .await
        .expect("a sentinel connection");
    client.query::<String>(&cmd("PING")).await.expect("a reply");
    let auth = master.commands_named("AUTH");
    assert!(
        auth.iter()
            .any(|recorded| recorded.args == ["AUTH", "data-user", "data-token"]),
        "the master authenticates with the injected token"
    );
}

async fn assert_missing_source(client: RedisClient) {
    let error = open_error(&factory(None), &client).await;
    assert!(
        matches!(error, LiveClientError::CredentialSourceMissing),
        "a provider record without a source fails: {error:?}"
    );
    let endpoint_failure = endpoint_error(&factory(None), &client);
    assert!(matches!(
        endpoint_failure,
        LiveClientError::CredentialSourceMissing
    ));
}

#[tokio::test]
async fn a_record_needing_a_source_without_one_fails() {
    let mut direct = direct_fields("127.0.0.1", 6379);
    direct.credential_provider = true;
    assert_missing_source(RedisClient::Direct(direct)).await;

    let mut url = url_client("redis://127.0.0.1:6379/0".to_owned());
    url.credential_provider_settings = Some(CredentialProviderSettings::Explicit);
    assert_missing_source(RedisClient::Url(url)).await;

    let mut cluster = cluster_record(&[]);
    cluster.credential_provider = true;
    assert_missing_source(RedisClient::Cluster(cluster)).await;

    let mut sentinel = sentinel_record(&[], "mymaster");
    sentinel.connection_kwargs.credential_provider = true;
    assert_missing_source(RedisClient::Sentinel(sentinel)).await;
}

#[tokio::test]
async fn a_failing_source_fails_the_connection_without_its_text() {
    let server = FakeServer::start().await;
    let credential_source: Arc<dyn RedisCredentialSource> = Arc::new(FailingSource);
    let connected = factory(Some(credential_source))
        .open_async(&RedisClient::Direct(provider_fields(&server)), None)
        .await;
    match connected {
        Ok(_connection) => panic!("a failing source must fail the connection"),
        Err(error) => assert_no_secret(&error, "no credential is available"),
    }
}
