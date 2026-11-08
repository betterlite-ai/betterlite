use redis::Value;

use super::fake_server::{FakeServer, Reply};
use super::fixtures::{cluster_pair, cluster_record, connected, factory, key_in_slot_range};
use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::redis_client_factories::RedisClient;

fn command_names(server: &FakeServer) -> Vec<String> {
    server
        .commands()
        .into_iter()
        .map(|recorded| recorded.name)
        .collect()
}

#[tokio::test]
async fn a_transaction_pipeline_sends_multi_and_exec() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Bulk("value".to_owned()));
    let client = connected(&server).await;
    let mut pipeline = client.pipeline(true);
    pipeline.commands.cmd("SET").arg("key").arg("value");
    pipeline.commands.cmd("GET").arg("key");
    let values = pipeline.execute().await.expect("a transaction");
    assert_eq!(values, [Value::Okay, Value::BulkString(b"value".to_vec())]);
    let names = command_names(&server);
    assert!(names.contains(&"MULTI".to_owned()));
    assert!(names.contains(&"EXEC".to_owned()));
}

#[tokio::test]
async fn a_plain_pipeline_sends_no_multi() {
    let server = FakeServer::start().await;
    let client = connected(&server).await;
    let mut pipeline = client.pipeline(false);
    pipeline.commands.cmd("SET").arg("key").arg("value");
    pipeline.commands.cmd("SET").arg("other").arg("value");
    let values = pipeline.execute().await.expect("a pipeline");
    assert_eq!(values.len(), 2);
    let names = command_names(&server);
    assert!(!names.contains(&"MULTI".to_owned()));
    assert!(!names.contains(&"EXEC".to_owned()));
}

#[tokio::test]
async fn execute_returns_the_first_error() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Error("ERR first".to_owned()));
    server.script("DEL", Reply::Error("ERR second".to_owned()));
    let client = connected(&server).await;
    let mut pipeline = client.pipeline(false);
    pipeline.commands.cmd("SET").arg("key").arg("value");
    pipeline.commands.cmd("GET").arg("key");
    pipeline.commands.cmd("DEL").arg("key");
    let error = pipeline.execute().await.expect_err("a failing command");
    let text = error.to_string();
    assert!(text.contains("first") && !text.contains("second"), "{text}");
}

#[tokio::test]
async fn execute_settled_keeps_one_result_per_command() {
    let server = FakeServer::start().await;
    server.script("GET", Reply::Bulk("value".to_owned()));
    server.script("DEL", Reply::Error("ERR boom".to_owned()));
    server.script("TTL", Reply::Int(3));
    let client = connected(&server).await;
    let mut pipeline = client.pipeline(false);
    pipeline.commands.cmd("GET").arg("key");
    pipeline.commands.cmd("DEL").arg("key");
    pipeline.commands.cmd("TTL").arg("key");
    let results = pipeline
        .execute_settled()
        .await
        .expect("a settled pipeline");
    assert_eq!(results.len(), 3);
    results[0].as_ref().expect("a first result");
    results[1].as_ref().expect_err("a failed result");
    assert_eq!(results[2].as_ref().expect("a ttl"), &Value::Int(3));
}

#[tokio::test]
async fn a_cluster_pipeline_runs_commands_across_slots_in_order() {
    let (first, second) = cluster_pair().await;
    first.script("GET", Reply::EchoArgs);
    second.script("GET", Reply::EchoArgs);
    let client = factory(None)
        .open_async(&RedisClient::Cluster(cluster_record(&[&first])), None)
        .await
        .expect("a cluster connection");
    let low = key_in_slot_range(0, 8191);
    let high = key_in_slot_range(8192, 16383);
    let mut pipeline = client.pipeline(false);
    pipeline.commands.cmd("GET").arg(low);
    pipeline.commands.cmd("GET").arg(high);
    let values = pipeline.execute().await.expect("a cluster pipeline");
    assert_eq!(
        values,
        [
            Value::Array(vec![Value::BulkString(low.as_bytes().to_vec())]),
            Value::Array(vec![Value::BulkString(high.as_bytes().to_vec())]),
        ]
    );
    assert_eq!(first.commands_named("GET").len(), 1);
    assert_eq!(second.commands_named("GET").len(), 1);
}

#[tokio::test]
async fn a_cluster_settled_pipeline_keeps_failures_in_place() {
    let (first, second) = cluster_pair().await;
    first.script("GET", Reply::Error("ERR first".to_owned()));
    second.script("GET", Reply::Int(1));
    let client = factory(None)
        .open_async(&RedisClient::Cluster(cluster_record(&[&first])), None)
        .await
        .expect("a cluster connection");
    let low = key_in_slot_range(0, 8191);
    let high = key_in_slot_range(8192, 16383);
    let mut pipeline = client.pipeline(false);
    pipeline.commands.cmd("GET").arg(low);
    pipeline.commands.cmd("GET").arg(high);
    let results = pipeline
        .execute_settled()
        .await
        .expect("a settled pipeline");
    assert_eq!(results.len(), 2);
    results[0].as_ref().expect_err("a failed result");
    assert_eq!(results[1].as_ref().expect("a value"), &Value::Int(1));
}

#[tokio::test]
async fn a_cluster_transaction_pipeline_stays_on_one_node() {
    let (first, second) = cluster_pair().await;
    let client = factory(None)
        .open_async(&RedisClient::Cluster(cluster_record(&[&first])), None)
        .await
        .expect("a cluster connection");
    let key = key_in_slot_range(0, 8191);
    let mut pipeline = client.pipeline(true);
    pipeline.commands.cmd("SET").arg(key).arg("1");
    pipeline.commands.cmd("GET").arg(key);
    let values = pipeline.execute().await.expect("a transaction");
    assert_eq!(values.len(), 2);
    assert_eq!(first.commands_named("MULTI").len(), 1);
    assert_eq!(first.commands_named("EXEC").len(), 1);
    assert!(second.commands_named("MULTI").is_empty());
}
