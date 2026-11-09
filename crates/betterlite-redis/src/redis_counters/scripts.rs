use std::sync::Arc;

use redis::{ErrorKind, ServerErrorKind, Value, cmd};

use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::protocol::RedisScalar;
use crate::redis_client_reuse::transport::RedisTransport;
use crate::redis_live_client::LiveClientError;

use super::RedisCountersError;

pub struct RegisteredScript {
    pub source: String,
    pub sha1: String,
    pub transport: Arc<RedisTransport>,
    pub namespace: Option<String>,
}

impl RegisteredScript {
    pub async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<Value, RedisCountersError> {
        let namespaced: Vec<String> = keys
            .iter()
            .map(|key| {
                check_and_fix_namespace(self.namespace.as_deref(), Some(key.as_str()))
                    .unwrap_or_else(|| key.clone())
            })
            .collect();
        let command = self.eval_sha_command(&namespaced, args);
        let client = self.transport.async_client().await?;
        match client.query::<Value>(&command).await {
            Ok(value) => Ok(value),
            Err(LiveClientError::Redis(error))
                if error.kind() == ErrorKind::Server(ServerErrorKind::NoScript) =>
            {
                let mut load = cmd("SCRIPT");
                load.arg("LOAD").arg(&self.source);
                client.query::<()>(&load).await?;
                Ok(client.query::<Value>(&command).await?)
            }
            Err(error) => Err(RedisCountersError::Client(error)),
        }
    }

    fn eval_sha_command(&self, keys: &[String], args: &[RedisScalar]) -> redis::Cmd {
        let mut command = cmd("EVALSHA");
        command.arg(&self.sha1).arg(keys.len());
        for key in keys {
            command.arg(key);
        }
        for scalar in args {
            match scalar {
                RedisScalar::Text(text) => {
                    command.arg(text);
                }
                RedisScalar::Bytes(bytes) => {
                    command.arg(bytes);
                }
                RedisScalar::Float(value) => {
                    command.arg(*value);
                }
            }
        }
        command
    }
}
