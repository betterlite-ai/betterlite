mod tests;

use std::time::Duration;

use async_trait::async_trait;
use betterlite_redis::cache_primitives::namespace::check_and_fix_namespace;
use betterlite_redis::redis_live_client::{LiveAsyncClient, LiveClientError};
use redis::{Cmd, Value};

const RELEASE_IF_OWNER: &str = "if redis.call('get', KEYS[1]) == ARGV[1] then return redis.call('del', KEYS[1]) else return 0 end";
const EXTEND_IF_OWNER: &str = "if redis.call('get', KEYS[1]) == ARGV[1] then return redis.call('pexpire', KEYS[1], ARGV[2]) else return 0 end";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockAcquisition {
    Acquired,
    Held,
    Error,
}

#[async_trait]
pub trait DistributedLock: Send + Sync {
    async fn acquire(&self, key: &str, token: &str, ttl: Duration) -> LockAcquisition;
    async fn extend(&self, key: &str, token: &str, ttl: Duration) -> bool;
    async fn release(&self, key: &str, token: &str);
    async fn is_held(&self, key: &str) -> bool;
}

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Live(#[from] LiveClientError),
}

#[async_trait]
pub trait RedisCommands: Send + Sync {
    async fn run(&self, command: &Cmd) -> Result<Value, CommandError>;
}

#[async_trait]
impl RedisCommands for LiveAsyncClient {
    // The live client needs a running Redis, which the unit tests fake at the RedisCommands seam.
    #[cfg_attr(test, mutants::skip)]
    async fn run(&self, command: &Cmd) -> Result<Value, CommandError> {
        Ok(self.query(command).await?)
    }
}

pub struct RedisDistributedLock<Client> {
    pub client: Client,
    pub namespace: Option<String>,
}

impl<Client: RedisCommands> RedisDistributedLock<Client> {
    fn namespaced(&self, key: &str) -> String {
        check_and_fix_namespace(self.namespace.as_deref(), Some(key))
            .unwrap_or_else(|| key.to_owned())
    }

    async fn run_command(&self, command: &Cmd) -> Result<Value, CommandError> {
        let result = self.client.run(command).await;
        if let Err(error) = &result {
            tracing::warn!(%error, "redis distributed lock command failed");
        }
        result
    }

    fn eval_if_owner(&self, script: &'static str, key: &str, token: &str) -> Cmd {
        let mut command = Cmd::new();
        command
            .arg("EVAL")
            .arg(script)
            .arg(1)
            .arg(self.namespaced(key))
            .arg(token);
        command
    }
}

#[async_trait]
impl<Client: RedisCommands> DistributedLock for RedisDistributedLock<Client> {
    async fn acquire(&self, key: &str, token: &str, ttl: Duration) -> LockAcquisition {
        let mut command = Cmd::new();
        command
            .arg("SET")
            .arg(self.namespaced(key))
            .arg(token)
            .arg("NX")
            .arg("PX")
            .arg(ttl.as_millis());
        match self.run_command(&command).await {
            Ok(Value::Nil) => LockAcquisition::Held,
            Ok(_value) => LockAcquisition::Acquired,
            Err(_error) => LockAcquisition::Error,
        }
    }

    async fn extend(&self, key: &str, token: &str, ttl: Duration) -> bool {
        let mut command = self.eval_if_owner(EXTEND_IF_OWNER, key, token);
        command.arg(ttl.as_millis());
        matches!(self.run_command(&command).await, Ok(Value::Int(1)))
    }

    async fn release(&self, key: &str, token: &str) {
        let command = self.eval_if_owner(RELEASE_IF_OWNER, key, token);
        drop(self.run_command(&command).await);
    }

    async fn is_held(&self, key: &str) -> bool {
        let mut command = Cmd::new();
        command.arg("EXISTS").arg(self.namespaced(key));
        matches!(self.run_command(&command).await, Ok(Value::Int(count)) if count > 0)
    }
}
