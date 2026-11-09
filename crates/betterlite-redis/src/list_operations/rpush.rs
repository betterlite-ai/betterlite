use redis::Value;

use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::protocol::{AsyncRedisCommands as _, RedisScalar};

use super::commands::{ltrim_command, rpush_command};
use super::error::RedisListOperationsError;
use super::guard::Call;
use super::{RedisListOperations, RedisPipelineRpushOperation};

pub(crate) fn rpush_call(key: &str) -> Call {
    Call {
        name: "async_rpush",
        key: Some(key.to_owned()),
        log: "LiteLLM Redis Cache RPUSH: - Got exception from REDIS",
    }
}

pub(crate) fn trim_call(key: &str) -> Call {
    Call {
        name: "async_rpush_and_trim",
        key: Some(key.to_owned()),
        log: "LiteLLM Redis Cache RPUSH+LTRIM: - Got exception from REDIS",
    }
}

pub(crate) const fn rpush_pipeline_call() -> Call {
    Call {
        name: "async_rpush_pipeline",
        key: None,
        log: "LiteLLM Redis Caching: async_rpush_pipeline() - Got exception from REDIS",
    }
}

pub(crate) async fn rpush(
    lists: &RedisListOperations,
    key: &str,
    values: &[RedisScalar],
) -> Result<i64, RedisListOperationsError> {
    match lists.transport.async_client().await {
        Ok(client) => Ok(client.rpush(key, values).await?),
        Err(error) => Err(RedisListOperationsError::ClientReuse(error)),
    }
}

pub(crate) async fn rpush_and_trim(
    lists: &RedisListOperations,
    key: &str,
    values: &[RedisScalar],
    max_len: i64,
) -> Result<i64, RedisListOperationsError> {
    let start = max_len
        .checked_neg()
        .ok_or(RedisListOperationsError::TrimLength { max_len })?;
    let client = lists.transport.async_client().await?;
    let mut pipeline = client.pipeline(true);
    pipeline.commands.add_command(rpush_command(key, values));
    pipeline.commands.add_command(ltrim_command(key, start, -1));
    let results = pipeline.execute().await?;
    match results.first() {
        Some(Value::Int(length)) => Ok(*length),
        _ => Err(RedisListOperationsError::UnexpectedReply {
            operation: "async_rpush_and_trim",
        }),
    }
}

pub(crate) async fn rpush_pipeline(
    lists: &RedisListOperations,
    rpush_list: &[RedisPipelineRpushOperation],
) -> Result<Vec<i64>, RedisListOperationsError> {
    let client = lists.transport.async_client().await?;
    let mut pipeline = client.pipeline(false);
    for operation in rpush_list {
        let key =
            check_and_fix_namespace(lists.settings.namespace.as_deref(), Some(&operation.key))
                .unwrap_or_else(|| operation.key.clone());
        pipeline
            .commands
            .add_command(rpush_command(&key, &operation.values));
    }
    let results = pipeline.execute().await?;
    let mut lengths = Vec::with_capacity(results.len());
    for value in results {
        match value {
            Value::Int(length) => lengths.push(length),
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
