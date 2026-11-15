#![cfg(test)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_redis::redis_live_client::LiveClientError;
use redis::{Cmd, Value};

use super::{CommandError, DistributedLock, LockAcquisition, RedisCommands, RedisDistributedLock};

const EXTEND_SCRIPT: &str = "if redis.call('get', KEYS[1]) == ARGV[1] then return redis.call('pexpire', KEYS[1], ARGV[2]) else return 0 end";
const RELEASE_SCRIPT: &str = "if redis.call('get', KEYS[1]) == ARGV[1] then return redis.call('del', KEYS[1]) else return 0 end";
const KEY: &str = "lock:user:1";
const TOKEN: &str = "tok";

struct FakeRedis {
    replies: Mutex<VecDeque<Result<Value, CommandError>>>,
    commands: Mutex<Vec<Vec<String>>>,
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

fn backend_failure() -> CommandError {
    CommandError::Live(LiveClientError::InvalidSetting { field: "url" })
}

fn lock(
    replies: Vec<Result<Value, CommandError>>,
) -> (RedisDistributedLock<Arc<FakeRedis>>, Arc<FakeRedis>) {
    let redis = Arc::new(FakeRedis {
        replies: Mutex::new(VecDeque::from(replies)),
        commands: Mutex::new(Vec::new()),
    });
    (
        RedisDistributedLock {
            client: redis.clone(),
            namespace: None,
        },
        redis,
    )
}

fn namespaced(
    namespace: &str,
    replies: Vec<Result<Value, CommandError>>,
) -> (RedisDistributedLock<Arc<FakeRedis>>, Arc<FakeRedis>) {
    let (lock, redis) = lock(replies);
    let lock = RedisDistributedLock {
        client: lock.client,
        namespace: Some(namespace.to_owned()),
    };
    (lock, redis)
}

fn commands(redis: &FakeRedis) -> Vec<Vec<String>> {
    redis.commands.lock().unwrap().clone()
}

fn ttl() -> Duration {
    Duration::from_secs(10)
}

#[tokio::test]
async fn acquire_sets_the_key_with_nx_and_px() {
    let (lock, redis) = lock(vec![Ok(Value::Okay)]);
    assert_eq!(
        lock.acquire(KEY, TOKEN, ttl()).await,
        LockAcquisition::Acquired
    );
    assert_eq!(
        commands(&redis)[0],
        vec!["SET", KEY, TOKEN, "NX", "PX", "10000"]
    );
}

#[tokio::test]
async fn acquire_accepts_any_reply_other_than_nil() {
    let (lock, _redis) = lock(vec![Ok(Value::SimpleString("OK".to_owned()))]);
    assert_eq!(
        lock.acquire(KEY, TOKEN, ttl()).await,
        LockAcquisition::Acquired
    );
}

#[tokio::test]
async fn acquire_reports_held_when_the_key_exists() {
    let (lock, _redis) = lock(vec![Ok(Value::Nil)]);
    assert_eq!(lock.acquire(KEY, TOKEN, ttl()).await, LockAcquisition::Held);
}

#[tokio::test]
async fn acquire_reports_an_error_on_a_backend_failure() {
    let (lock, _redis) = lock(vec![Err(backend_failure())]);
    assert_eq!(
        lock.acquire(KEY, TOKEN, ttl()).await,
        LockAcquisition::Error
    );
}

#[tokio::test]
async fn extend_renews_only_for_the_owner() {
    let (lock, redis) = lock(vec![Ok(Value::Int(1))]);
    assert!(lock.extend(KEY, TOKEN, ttl()).await);
    assert_eq!(
        commands(&redis)[0],
        vec!["EVAL", EXTEND_SCRIPT, "1", KEY, TOKEN, "10000"]
    );
}

#[tokio::test]
async fn extend_reports_false_for_another_owner() {
    let (lock, _redis) = lock(vec![Ok(Value::Int(0))]);
    assert!(!lock.extend(KEY, TOKEN, ttl()).await);
}

#[tokio::test]
async fn extend_reports_false_for_any_other_reply() {
    let (lock, _redis) = lock(vec![Ok(Value::Nil)]);
    assert!(!lock.extend(KEY, TOKEN, ttl()).await);
}

#[tokio::test]
async fn extend_reports_false_on_a_backend_failure() {
    let (lock, _redis) = lock(vec![Err(backend_failure())]);
    assert!(!lock.extend(KEY, TOKEN, ttl()).await);
}

#[tokio::test]
async fn release_deletes_only_for_the_owner() {
    let (lock, redis) = lock(vec![Ok(Value::Int(1))]);
    lock.release(KEY, TOKEN).await;
    assert_eq!(
        commands(&redis)[0],
        vec!["EVAL", RELEASE_SCRIPT, "1", KEY, TOKEN]
    );
}

#[tokio::test]
async fn release_swallows_a_backend_failure() {
    let (lock, redis) = lock(vec![Err(backend_failure())]);
    lock.release(KEY, TOKEN).await;
    assert_eq!(commands(&redis).len(), 1);
}

#[tokio::test]
async fn is_held_asks_exists() {
    let (lock, redis) = lock(vec![Ok(Value::Int(2))]);
    assert!(lock.is_held(KEY).await);
    assert_eq!(commands(&redis)[0], vec!["EXISTS", KEY]);
}

#[tokio::test]
async fn is_held_is_false_for_a_missing_key() {
    let (lock, _redis) = lock(vec![Ok(Value::Int(0))]);
    assert!(!lock.is_held(KEY).await);
}

#[tokio::test]
async fn is_held_is_false_for_any_other_reply() {
    let (lock, _redis) = lock(vec![Ok(Value::Nil)]);
    assert!(!lock.is_held(KEY).await);
}

#[tokio::test]
async fn is_held_is_false_on_a_backend_failure() {
    let (lock, _redis) = lock(vec![Err(backend_failure())]);
    assert!(!lock.is_held(KEY).await);
}

#[tokio::test]
async fn every_key_carries_the_namespace() {
    let (lock, redis) = namespaced("ns", vec![Ok(Value::Okay), Ok(Value::Int(1))]);
    assert_eq!(
        lock.acquire(KEY, TOKEN, ttl()).await,
        LockAcquisition::Acquired
    );
    lock.release(KEY, TOKEN).await;
    let sent = commands(&redis);
    assert_eq!(sent[0][1], "ns:lock:user:1");
    assert_eq!(sent[1][3], "ns:lock:user:1");
}

#[tokio::test]
async fn an_already_prefixed_key_keeps_one_namespace() {
    let (lock, redis) = namespaced("ns", vec![Ok(Value::Int(0))]);
    assert!(!lock.is_held("ns:already-prefixed").await);
    assert_eq!(commands(&redis)[0], vec!["EXISTS", "ns:already-prefixed"]);
}

#[tokio::test]
async fn an_empty_namespace_leaves_keys_bare() {
    let (lock, redis) = namespaced("", vec![Ok(Value::Int(0))]);
    assert!(!lock.is_held(KEY).await);
    assert_eq!(commands(&redis)[0], vec!["EXISTS", KEY]);
}

#[tokio::test]
async fn the_ttl_goes_to_redis_in_milliseconds() {
    let (lock, redis) = lock(vec![Ok(Value::Okay), Ok(Value::Int(1))]);
    lock.acquire(KEY, TOKEN, Duration::from_millis(1500)).await;
    lock.extend(KEY, TOKEN, Duration::from_millis(1)).await;
    let sent = commands(&redis);
    assert_eq!(sent[0][5], "1500");
    assert_eq!(sent[1][5], "1");
}
