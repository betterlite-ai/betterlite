#![cfg(test)]

use self::fixtures::fakes::{
    reader_unavailable, set_reader_unavailable, set_writer_unavailable, writer_unavailable,
};
use crate::database_replica::client::DatabaseLifecycle as _;
use crate::database_replica::pinned::{WriterPinnedClient, pin_writer, writer_wrapper};

use self::fixtures::postgres::{
    close_replica_case, missing_database_url, replica_case, routing_over,
};

mod dispatch;
mod fixtures;
mod lifecycle;

#[tokio::test]
async fn postgres_reader_dispatch() {
    let case = replica_case("writer", "reader").await;
    let routing = routing_over(&case);
    let route = routing
        .attribute("ReplicaRow")
        .expect("the model attribute routes");
    assert_eq!(route.select("find_many").notes().await, ["reader"]);
    set_reader_unavailable(&routing, true);
    let writer_route = routing
        .attribute("ReplicaRow")
        .expect("the model attribute routes");
    assert_eq!(writer_route.select("find_many").notes().await, ["writer"]);
    Box::pin(close_replica_case(case, &routing)).await;
}

#[tokio::test]
async fn postgres_writer_read_after_write() {
    let case = replica_case("writer", "stale").await;
    let routing = routing_over(&case);
    let writer = writer_wrapper(WriterPinnedClient::Routed(&routing));
    let mut client = writer.pool().get().await.expect("a writer connection");
    let transaction = client
        .transaction()
        .await
        .expect("a writer transaction begins");
    transaction
        .execute("INSERT INTO replica_rows VALUES ('fresh')", &[])
        .await
        .expect("the fresh row is written");
    let row = transaction
        .query_one("SELECT note FROM replica_rows WHERE note = 'fresh'", &[])
        .await
        .expect("the transaction reads its own write");
    assert_eq!(row.get::<_, String>(0), "fresh");
    transaction.commit().await.expect("the transaction commits");
    let route = routing
        .attribute("ReplicaRow")
        .expect("the model attribute routes");
    assert_eq!(route.select("find_many").notes().await, ["stale"]);
    assert_eq!(routing.writer.model().notes().await, ["fresh", "writer"]);
    Box::pin(close_replica_case(case, &routing)).await;
}

#[tokio::test]
async fn postgres_degraded_writer_model_read() {
    let case = replica_case("writer", "reader").await;
    let routing = routing_over(&case);
    routing.writer.pool().close();
    set_writer_unavailable(&routing, true);
    assert!(!routing.is_connected());
    let pinned = pin_writer(WriterPinnedClient::Routed(&routing));
    assert!(matches!(pinned, WriterPinnedClient::Routed(_)));
    let route = pinned
        .attribute("ReplicaRow")
        .expect("the routed view serves the model");
    assert_eq!(route.select("find_many").notes().await, ["reader"]);
    Box::pin(close_replica_case(case, &routing)).await;
}

#[tokio::test]
async fn postgres_reader_failure_preserves_writer_commit() {
    let case = replica_case("committed", "stale").await;
    let mut routing = routing_over(&case);
    routing.reader_url = Some(missing_database_url(&case.reader.url));
    let url = case.writer.url.clone();
    let recreated = routing
        .recreate(&url, None, None)
        .await
        .expect("the writer replacement succeeds");
    assert!(recreated);
    assert!(reader_unavailable(&routing));
    assert!(!writer_unavailable(&routing));
    assert_eq!(routing.writer.model().notes().await, ["committed"]);
    Box::pin(close_replica_case(case, &routing)).await;
}
