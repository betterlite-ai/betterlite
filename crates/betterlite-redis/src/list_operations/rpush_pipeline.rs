use redis::Value;

use crate::cache_primitives::protocol::AsyncRedisCommands as _;

use super::error::RedisListOperationsError;
use super::rpush::rpush_command;
use super::{Call, ListResult, RedisListOperations, RedisPipelineRpushOperation, namespaced_key};

pub(crate) async fn async_rpush_pipeline(
    lists: &RedisListOperations,
    call: Call,
    rpush_list: &[RedisPipelineRpushOperation],
) -> ListResult<Vec<i64>> {
    let Some(admission) = lists.admit(call.name, rpush_list.is_empty())? else {
        return Ok(Vec::new());
    };
    let request = async {
        let client = lists.transport.async_client().await?;
        let mut pipeline = client.pipeline(false);
        for operation in rpush_list {
            let key = namespaced_key(lists, &operation.key);
            pipeline
                .commands
                .add_command(rpush_command(&key, &operation.values));
        }
        let results = pipeline.execute().await?;
        lengths(&results)
    };
    lists.perform(admission, call, None, request).await
}

fn lengths(results: &[Value]) -> ListResult<Vec<i64>> {
    let mut lengths = Vec::with_capacity(results.len());
    for value in results {
        match value {
            Value::Int(length) => lengths.push(*length),
            Value::Nil
            | Value::BulkString(_)
            | Value::Array(_)
            | Value::SimpleString(_)
            | Value::Okay
            | Value::Map(_)
            | Value::Attribute { .. }
            | Value::Set(_)
            | Value::Double(_)
            | Value::Boolean(_)
            | Value::VerbatimString { .. }
            | Value::BigNumber(_)
            | Value::Push { .. }
            | Value::ServerError(_)
            | _ => {
                return Err(RedisListOperationsError::UnexpectedReply {
                    operation: "async_rpush_pipeline",
                });
            }
        }
    }
    Ok(lengths)
}
