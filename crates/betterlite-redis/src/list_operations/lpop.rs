use std::str;

use redis::Value;

use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::cache_primitives::version::parse_major_version;
use crate::redis_live_client::LiveAsyncClient;

use super::commands::lpop_command;
use super::error::RedisListOperationsError;
use super::guard::Call;
use super::namespaced_key;
use super::results::{
    LpopPipelineEntry, LpopResult, decode_group, decode_popped, decode_reply, popped_items,
    reply_bytes,
};
use super::{RedisListOperations, RedisPipelineLpopOperation};

pub(crate) fn lpop_call(key: &str) -> Call {
    Call {
        name: "async_lpop",
        key: Some(key.to_owned()),
        log: "LiteLLM Redis Cache LPOP: - Got exception from REDIS",
    }
}

pub(crate) const fn lpop_pipeline_call() -> Call {
    Call {
        name: "async_lpop_pipeline",
        key: None,
        log: "LiteLLM Redis Caching: async_lpop_pipeline() - Got exception from REDIS",
    }
}

pub(crate) async fn lpop(
    lists: &RedisListOperations,
    key: &str,
    count: Option<i64>,
) -> Result<LpopResult, RedisListOperationsError> {
    let client = lists.transport.async_client().await?;
    let major = parse_major_version(
        &lists.settings.redis_version,
        lists.settings.default_major_version,
    );
    let Some(limit) = count.filter(|_value| major < 7) else {
        let reply = client.lpop(key, count).await?;
        return decode_reply(&reply);
    };
    let popped = pop_in_loop(&client, key, limit).await?;
    Ok(decode_popped(&popped))
}

async fn pop_in_loop(
    client: &LiveAsyncClient,
    key: &str,
    count: i64,
) -> Result<Vec<Vec<u8>>, RedisListOperationsError> {
    let mut popped = Vec::new();
    for _ in 0..count {
        let mut pipeline = client.pipeline(false);
        pipeline.commands.add_command(lpop_command(key, None));
        for value in pipeline.execute().await? {
            if let Some(bytes) = reply_bytes(&value, "async_lpop")? {
                popped.push(bytes.to_vec());
            }
        }
    }
    Ok(popped)
}

pub(crate) async fn lpop_pipeline(
    lists: &RedisListOperations,
    lpop_list: &[RedisPipelineLpopOperation],
) -> Result<Vec<LpopPipelineEntry>, RedisListOperationsError> {
    let client = lists.transport.async_client().await?;
    let major = parse_major_version(
        &lists.settings.redis_version,
        lists.settings.default_major_version,
    );
    if major >= 7 {
        return bulk_pipeline(lists, &client, lpop_list).await;
    }
    grouped_pipeline(lists, &client, lpop_list).await
}

async fn bulk_pipeline(
    lists: &RedisListOperations,
    client: &LiveAsyncClient,
    lpop_list: &[RedisPipelineLpopOperation],
) -> Result<Vec<LpopPipelineEntry>, RedisListOperationsError> {
    let mut pipeline = client.pipeline(false);
    for operation in lpop_list {
        let key = namespaced_key(lists, &operation.key);
        pipeline
            .commands
            .add_command(lpop_command(&key, operation.count));
    }
    let results = pipeline.execute().await?;
    let mut entries = Vec::with_capacity(results.len());
    for value in &results {
        entries.push(bulk_entry(value)?);
    }
    Ok(entries)
}

// litellm queues one LPOP per count and regroups the flat reply, so one key's empty list shifts no other key
async fn grouped_pipeline(
    lists: &RedisListOperations,
    client: &LiveAsyncClient,
    lpop_list: &[RedisPipelineLpopOperation],
) -> Result<Vec<LpopPipelineEntry>, RedisListOperationsError> {
    let planned: Vec<(String, i64)> = lpop_list
        .iter()
        .map(|operation| {
            (
                namespaced_key(lists, &operation.key),
                operation.count.unwrap_or(1),
            )
        })
        .collect();
    let mut pipeline = client.pipeline(false);
    let mut counts = Vec::with_capacity(planned.len());
    for (key, count) in &planned {
        counts.push(*count);
        for _ in 0..*count {
            pipeline.commands.add_command(lpop_command(key, None));
        }
    }
    let flat = pipeline.execute().await?;
    let mut values = flat.into_iter();
    let mut entries = Vec::with_capacity(counts.len());
    for count in counts {
        let mut group = Vec::new();
        for _ in 0..count {
            let Some(value) = values.next() else {
                break;
            };
            if let Some(bytes) = reply_bytes(&value, "async_lpop_pipeline")? {
                group.push(bytes.to_vec());
            }
        }
        entries.push(decode_group(&group));
    }
    Ok(entries)
}

fn bulk_entry(value: &Value) -> Result<LpopPipelineEntry, RedisListOperationsError> {
    if let Value::Array(items) = value {
        return Ok(decode_group(&popped_items(items, "async_lpop_pipeline")?));
    }
    let Some(bytes) = reply_bytes(value, "async_lpop_pipeline")? else {
        return Ok(LpopPipelineEntry::Missing);
    };
    Ok(single_entry(bytes))
}

fn single_entry(bytes: &[u8]) -> LpopPipelineEntry {
    match str::from_utf8(bytes) {
        Ok(text) => LpopPipelineEntry::Items(vec![text.to_owned()]),
        Err(_source) => LpopPipelineEntry::ItemsBytes(vec![bytes.to_vec()]),
    }
}
