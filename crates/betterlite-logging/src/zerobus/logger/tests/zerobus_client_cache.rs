use std::sync::Arc;

use secrecy::SecretString;

use crate::zerobus::logger::ZerobusLogger;
use crate::zerobus::logger::connection::{ConnectionError, SecretSource};
use crate::zerobus::logger::delivery::{RowIngest, same_connection};

use super::support::{
    FakeIngest, MapSecrets, as_row_ingest, as_secret_source, connection, connection_secrets, deps,
    logger, params,
};

fn cached_logger() -> (ZerobusLogger, Arc<MapSecrets>) {
    let secrets = Arc::new(connection_secrets());
    let source: Arc<dyn SecretSource> = as_secret_source(&secrets);
    let mut configured = deps(params(100, 3600, false), None);
    configured.secrets = source;
    let logger = ZerobusLogger::new(configured).expect("the connection resolves");
    (logger, secrets)
}

#[tokio::test]
async fn zerobus_client_cache_returns_the_given_client() {
    let ingest = FakeIngest::table("main.litellm.traces");
    let given: Arc<dyn RowIngest> = as_row_ingest(&ingest);
    let logger = logger(params(100, 3600, false), Arc::clone(&given));
    let returned = logger.inner.client().expect("the given client is returned");
    assert!(Arc::ptr_eq(&returned, &given));
}

#[tokio::test]
async fn zerobus_client_cache_reuses_the_client_for_the_same_connection() {
    let (logger, _secrets) = cached_logger();
    let first = logger.inner.client().expect("the first client builds");
    let second = logger
        .inner
        .client()
        .expect("the cached client is returned");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.connection().table_name, "main.litellm.traces");
}

#[tokio::test]
async fn zerobus_client_cache_rebuilds_when_the_table_changes() {
    let (logger, secrets) = cached_logger();
    let first = logger.inner.client().expect("the first client builds");
    secrets.set("ZEROBUS_TABLE_NAME", "main.litellm.other");
    let second = logger.inner.client().expect("the client is rebuilt");
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(first.connection().table_name, "main.litellm.traces");
    assert_eq!(second.connection().table_name, "main.litellm.other");
}

#[tokio::test]
async fn zerobus_client_cache_rebuilds_when_the_secret_changes() {
    let (logger, secrets) = cached_logger();
    let first = logger.inner.client().expect("the first client builds");
    secrets.set("ZEROBUS_CLIENT_SECRET", "rotated-secret");
    let second = logger.inner.client().expect("the client is rebuilt");
    assert!(!Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn zerobus_client_cache_reports_a_missing_setting_at_init() {
    let configured = deps(params(100, 3600, false), None);
    let error = ZerobusLogger::new(configured)
        .err()
        .expect("the connection is incomplete");
    assert!(matches!(
        error,
        ConnectionError::Missing { variable, .. } if variable == "ZEROBUS_SERVER_ENDPOINT"
    ));
}

#[tokio::test]
async fn zerobus_client_cache_reports_a_setting_that_later_disappears() {
    let (logger, secrets) = cached_logger();
    secrets.set("ZEROBUS_TABLE_NAME", "");
    let error = logger
        .inner
        .client()
        .err()
        .expect("the table is missing now");
    assert!(matches!(
        error,
        ConnectionError::Missing { variable, .. } if variable == "ZEROBUS_TABLE_NAME"
    ));
}

#[test]
fn zerobus_client_cache_compares_every_connection_field() {
    let base = connection("main.litellm.traces");
    assert!(same_connection(&base, &base.clone()));

    let mut other_url = base.clone();
    other_url.workspace_url = "https://other.cloud.databricks.com".to_owned();
    assert!(!same_connection(&base, &other_url));

    let mut other_workspace = base.clone();
    other_workspace.workspace_id = "999".to_owned();
    assert!(!same_connection(&base, &other_workspace));

    let mut other_endpoint = base.clone();
    other_endpoint.server_endpoint =
        "https://999.zerobus.us-west-2.cloud.databricks.com".to_owned();
    assert!(!same_connection(&base, &other_endpoint));

    let mut other_client = base.clone();
    other_client.client_id = "other-client".to_owned();
    assert!(!same_connection(&base, &other_client));

    let mut other_secret = base.clone();
    other_secret.client_secret = SecretString::from("other-secret");
    assert!(!same_connection(&base, &other_secret));

    let mut other_table = base.clone();
    other_table.table_name = "main.litellm.other".to_owned();
    assert!(!same_connection(&base, &other_table));

    let same = connection("main.litellm.traces");
    assert!(same_connection(&base, &same));
}
