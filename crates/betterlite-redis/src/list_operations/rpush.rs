use redis::{Cmd, cmd};

use crate::cache_primitives::protocol::{AsyncRedisCommands as _, RedisScalar};

use super::error::RedisListOperationsError;
use super::{Call, ListResult, RedisListOperations, namespaced_key};

pub(crate) fn rpush_command(key: &str, values: &[RedisScalar]) -> Cmd {
    let mut command = cmd("RPUSH");
    command.arg(key);
    for value in values {
        match value {
            RedisScalar::Text(text) => command.arg(text),
            RedisScalar::Bytes(bytes) => command.arg(bytes),
            RedisScalar::Float(number) => command.arg(*number),
        };
    }
    command
}

pub(crate) async fn async_rpush(
    lists: &RedisListOperations,
    call: Call,
    key: &str,
    values: &[RedisScalar],
) -> ListResult<i64> {
    let admission = lists.gate.enter(call.name)?;
    let request = async {
        let client = lists.transport.async_client().await?;
        let namespaced = namespaced_key(lists, key);
        client
            .rpush(&namespaced, values)
            .await
            .map_err(RedisListOperationsError::from)
    };
    lists.perform(admission, call, Some(key), request).await
}
