use redis::{Cmd, Value, cmd};

use super::pipeline::LivePipeline;
use super::{LiveAsyncClient, LiveClientError};
use crate::cache_primitives::protocol::{AsyncRedisCommands, RedisScalar};

fn push_scalar(command: &mut Cmd, scalar: &RedisScalar) {
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

impl AsyncRedisCommands for LiveAsyncClient {
    type Error = LiveClientError;
    type Reply = Value;
    type Pipeline = LivePipeline;

    async fn ping(&self) -> Result<bool, Self::Error> {
        let reply: String = self.query(&cmd("PING")).await?;
        Ok(reply == "PONG")
    }

    async fn delete(&self, names: &[&str]) -> Result<i64, Self::Error> {
        let mut command = cmd("DEL");
        command.arg(names);
        self.query(&command).await
    }

    async fn ttl(&self, name: &str) -> Result<i64, Self::Error> {
        let mut command = cmd("TTL");
        command.arg(name);
        self.query(&command).await
    }

    async fn expire(&self, name: &str, time: i64) -> Result<bool, Self::Error> {
        let mut command = cmd("EXPIRE");
        command.arg(name).arg(time);
        self.query(&command).await
    }

    async fn rpush(&self, name: &str, values: &[RedisScalar]) -> Result<i64, Self::Error> {
        let mut command = cmd("RPUSH");
        command.arg(name);
        for value in values {
            push_scalar(&mut command, value);
        }
        self.query(&command).await
    }

    async fn lpop(&self, name: &str, count: Option<i64>) -> Result<Self::Reply, Self::Error> {
        let mut command = cmd("LPOP");
        command.arg(name);
        if let Some(limit) = count {
            command.arg(limit);
        }
        self.query(&command).await
    }

    fn pipeline(&self, transaction: bool) -> Self::Pipeline {
        LivePipeline::new(self.clone(), transaction)
    }

    async fn eval(
        &self,
        script: &str,
        numkeys: i64,
        keys_and_args: &[RedisScalar],
    ) -> Result<Self::Reply, Self::Error> {
        let mut command = cmd("EVAL");
        command.arg(script).arg(numkeys);
        for value in keys_and_args {
            push_scalar(&mut command, value);
        }
        self.query(&command).await
    }
}
