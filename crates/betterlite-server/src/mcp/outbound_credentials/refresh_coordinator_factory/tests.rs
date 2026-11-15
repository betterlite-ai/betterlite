#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_redis::redis_live_client::LiveClientError;
use redis::{Cmd, Value};

use super::runtime_refresh_coordinator;
use crate::mcp::outbound_credentials::distributed_lock::{CommandError, RedisCommands};
use crate::mcp::outbound_credentials::refresh_coordinator::RedisTunables;
use crate::mcp::outbound_credentials::{RefreshCoordinator, RefreshStep, TokenKey};

struct FakeRedis {
    replies: Mutex<VecDeque<Result<Value, CommandError>>>,
    commands: Mutex<Vec<Vec<String>>>,
}

impl FakeRedis {
    fn new(replies: Vec<Result<Value, CommandError>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            commands: Mutex::new(Vec::new()),
        })
    }

    fn commands(&self) -> Vec<Vec<String>> {
        self.commands.lock().unwrap().clone()
    }
}

#[async_trait]
impl RedisCommands for Arc<FakeRedis> {
    async fn run(&self, command: &Cmd) -> Result<Value, CommandError> {
        let packed = command.get_packed_command();
        self.commands.lock().unwrap().push(read_command(&packed));
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(Value::Nil))
    }
}

// The packed command is a RESP array of bulk strings; every value here is plain text.
fn read_command(packed: &[u8]) -> Vec<String> {
    let text = std::str::from_utf8(packed).unwrap();
    text.split("\r\n")
        .filter(|part| !part.is_empty() && !part.starts_with('*') && !part.starts_with('$'))
        .map(str::to_owned)
        .collect()
}

fn key() -> TokenKey {
    TokenKey {
        user_id: "user-1".to_owned(),
        server_id: "server-1".to_owned(),
    }
}

fn refresh() -> RefreshStep<'static, String, String> {
    Box::new(|| Box::pin(async { Ok("fresh".to_owned()) }))
}

fn reread() -> RefreshStep<'static, String, String> {
    Box::new(|| Box::pin(async { Ok("reread".to_owned()) }))
}

fn build(
    client: Option<Arc<FakeRedis>>,
    namespace: Option<&str>,
    tunables: &RedisTunables,
) -> Option<Arc<dyn RefreshCoordinator<Output = String, Error = String>>> {
    runtime_refresh_coordinator(client, namespace, tunables)
}

#[test]
fn no_client_means_no_coordinator() {
    assert!(build(None, None, &RedisTunables::default()).is_none());
}

#[tokio::test]
async fn a_client_gives_a_coordinator_that_locks_under_the_namespace() {
    let redis = FakeRedis::new(vec![Ok(Value::Okay), Ok(Value::Int(1))]);
    let coordinator = build(Some(redis.clone()), Some("ns"), &RedisTunables::default()).unwrap();
    let result = coordinator.run(key(), refresh(), reread()).await;
    assert_eq!(result, Ok("fresh".to_owned()));
    let sent = redis.commands();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0][0], "SET");
    assert_eq!(sent[0][1], "ns:mcp:refresh_lock:user-1:server-1");
    assert_eq!(sent[0][2].len(), 32);
    assert_eq!(&sent[0][3..], ["NX", "PX", "10000"]);
    assert_eq!(sent[1][0], "EVAL");
    assert_eq!(sent[1][3], "ns:mcp:refresh_lock:user-1:server-1");
}

#[tokio::test]
async fn the_tunables_travel_with_the_coordinator() {
    let mut tunables = RedisTunables::default();
    tunables.key_prefix = "shared:".to_owned();
    tunables.lock_ttl = Duration::from_millis(250);
    let redis = FakeRedis::new(vec![Ok(Value::Okay), Ok(Value::Int(1))]);
    let coordinator = build(Some(redis.clone()), None, &tunables).unwrap();
    let result = coordinator.run(key(), refresh(), reread()).await;
    assert_eq!(result, Ok("fresh".to_owned()));
    let sent = redis.commands();
    assert_eq!(sent[0][1], "shared:user-1:server-1");
    assert_eq!(sent[0][5], "250");
}

#[tokio::test]
async fn a_held_lock_rereads_after_the_holder_lets_go() {
    let redis = FakeRedis::new(vec![Ok(Value::Nil), Ok(Value::Int(0))]);
    let coordinator = build(Some(redis.clone()), None, &RedisTunables::default()).unwrap();
    let result = coordinator.run(key(), refresh(), reread()).await;
    assert_eq!(result, Ok("reread".to_owned()));
    let sent = redis.commands();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1], vec!["EXISTS", "mcp:refresh_lock:user-1:server-1"]);
}

#[tokio::test]
async fn an_acquire_error_still_refreshes() {
    let redis = FakeRedis::new(vec![Err(CommandError::Live(
        LiveClientError::InvalidSetting { field: "url" },
    ))]);
    let coordinator = build(Some(redis.clone()), None, &RedisTunables::default()).unwrap();
    let result = coordinator.run(key(), refresh(), reread()).await;
    assert_eq!(result, Ok("fresh".to_owned()));
    assert_eq!(redis.commands().len(), 1);
}
