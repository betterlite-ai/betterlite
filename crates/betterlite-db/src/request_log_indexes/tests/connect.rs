use std::time::Duration;

use deadpool_postgres::GenericClient as _;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::task::JoinSet;
use tokio_postgres::NoTls;
use url::Url;

use crate::request_log_indexes::RequestLogIndexError;
use crate::request_log_indexes::connect::{self, CONNECT_TIMEOUT, connection_options};

use super::support::{PostgresTest, stalled_database_url};

#[tokio::test]
async fn connect_times_out_on_a_stalled_server() {
    let (url, mut sockets) = stalled_database_url().await;
    let outcome = connect::connect(&url, Duration::from_millis(200)).await;
    assert!(matches!(outcome, Err(RequestLogIndexError::ConnectTimeout)));
    sockets.shutdown().await;
}

#[tokio::test]
async fn connect_timeout_ten_autocommit() {
    let options =
        connection_options("postgresql://user:secret@127.0.0.1:5432/database?sslmode=disable")
            .unwrap();
    assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(10));
    assert_eq!(options.config.get_connect_timeout(), Some(&CONNECT_TIMEOUT));

    let test = PostgresTest::new().await;
    let database = test.service().await;
    let pid: i32 = database
        .client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .unwrap()
        .get(0);
    database.client.batch_execute("SELECT 1").await.unwrap();
    let (observer, mut drivers) = test.another_client().await;
    let state: String = observer
        .query_one("SELECT state FROM pg_stat_activity WHERE pid = $1", &[&pid])
        .await
        .unwrap()
        .get(0);
    assert_eq!(state, "idle");
    drivers.shutdown().await;
    test.finish().await;
}

#[tokio::test]
async fn statement_timeout_disabled() {
    let test = PostgresTest::new().await;
    let url = with_statement_timeout(&test.url, "250ms");
    let database = connect::connect(&url, CONNECT_TIMEOUT).await.unwrap();
    let configured: String = database
        .client
        .query_one("SHOW statement_timeout", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(configured, "0");

    let (raw, connection) = tokio_postgres::connect(url.expose_secret(), NoTls)
        .await
        .unwrap();
    let mut drivers = JoinSet::new();
    drivers.spawn(connection);
    let default: String = raw
        .query_one("SHOW statement_timeout", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(default, "250ms");
    drivers.shutdown().await;
    test.finish().await;
}

fn with_statement_timeout(url: &SecretString, value: &str) -> SecretString {
    let mut rebuilt = Url::parse(url.expose_secret()).unwrap();
    let pairs: Vec<(String, String)> = rebuilt.query_pairs().into_owned().collect();
    let options = pairs
        .iter()
        .find(|(key, _)| key == "options")
        .map(|(_, original)| original.clone())
        .unwrap_or_default();
    rebuilt.set_query(None);
    let widened = format!("{options} -c statement_timeout={value}");
    rewrite_query(&mut rebuilt, &pairs, &widened);
    let encoded = rebuilt.query().unwrap_or_default().replace('+', "%20");
    rebuilt.set_query(Some(&encoded));
    SecretString::from(rebuilt.to_string())
}

fn rewrite_query(rebuilt: &mut Url, pairs: &[(String, String)], options: &str) {
    let mut query = rebuilt.query_pairs_mut();
    for (key, original) in pairs.iter().filter(|(key, _)| key != "options") {
        query.append_pair(key, original);
    }
    query.append_pair("options", options);
    query.finish();
}
