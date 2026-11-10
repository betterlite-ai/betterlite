use redis::{Cmd, Value, cmd};

use crate::cache_primitives::protocol::{AsyncRedisCommands as _, RedisScalar};

use super::error::RedisListOperationsError;
use super::rpush::rpush_command;
use super::{Call, ListResult, RedisListOperations, namespaced_key};

pub(crate) fn ltrim_command(key: &str, start: i64, stop: i64) -> Cmd {
    let mut command = cmd("LTRIM");
    command.arg(key).arg(start).arg(stop);
    command
}

pub(crate) async fn async_rpush_and_trim(
    lists: &RedisListOperations,
    call: Call,
    key: &str,
    values: &[RedisScalar],
    max_len: i64,
) -> ListResult<i64> {
    let admission = lists.gate.enter(call.name)?;
    let request = async {
        let namespaced = namespaced_key(lists, key);
        let start = max_len
            .checked_neg()
            .ok_or(RedisListOperationsError::TrimLength { max_len })?;
        let client = lists.transport.async_client().await?;
        let mut pipeline = client.pipeline(true);
        pipeline
            .commands
            .add_command(rpush_command(&namespaced, values));
        pipeline
            .commands
            .add_command(ltrim_command(&namespaced, start, -1));
        let results = pipeline.execute().await?;
        match results.first() {
            Some(Value::Int(length)) => Ok(*length),
            _ => Err(RedisListOperationsError::UnexpectedReply {
                operation: "async_rpush_and_trim",
            }),
        }
    };
    lists.perform(admission, call, Some(key), request).await
}
