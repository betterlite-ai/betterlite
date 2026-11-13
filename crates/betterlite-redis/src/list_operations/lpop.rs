use std::str;

use redis::{Cmd, Value, cmd};

use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::cache_primitives::version::parse_major_version;
use crate::redis_live_client::LiveAsyncClient;

use super::error::RedisListOperationsError;
use super::lpop_grouped;
use super::results::{
    LpopPipelineEntry, LpopResult, decode_group, decode_popped, decode_reply, popped_items,
    reply_bytes,
};
use super::{Call, ListResult, namespaced_key};
use super::{RedisListOperations, RedisPipelineLpopOperation};

pub(crate) fn lpop_command(key: &str, count: Option<i64>) -> Cmd {
    let mut command = cmd("LPOP");
    command.arg(key);
    if let Some(limit) = count {
        command.arg(limit);
    }
    command
}

pub(crate) async fn async_lpop(
    lists: &RedisListOperations,
    call: Call,
    key: &str,
    count: Option<i64>,
) -> ListResult<LpopResult> {
    let admission = lists.gate.enter(call.name)?;
    let request = async {
        let namespaced = namespaced_key(lists, key);
        let client = lists.transport.async_client().await?;
        let major = parse_major_version(
            &lists.settings.redis_version,
            lists.settings.default_major_version,
        );
        let Some(limit) = count.filter(|_value| major < 7) else {
            let reply = client.lpop(&namespaced, count).await?;
            return decode_reply(&reply);
        };
        let popped = pop_in_loop(&client, &namespaced, limit).await?;
        Ok(decode_popped(&popped))
    };
    lists.perform(admission, call, Some(key), request).await
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

pub(crate) async fn async_lpop_pipeline(
    lists: &RedisListOperations,
    call: Call,
    lpop_list: &[RedisPipelineLpopOperation],
) -> ListResult<Vec<LpopPipelineEntry>> {
    let Some(admission) = lists.admit(call.name, lpop_list.is_empty())? else {
        return Ok(Vec::new());
    };
    let request = async {
        let client = lists.transport.async_client().await?;
        let major = parse_major_version(
            &lists.settings.redis_version,
            lists.settings.default_major_version,
        );
        if major >= 7 {
            return bulk_pipeline(lists, &client, lpop_list).await;
        }
        lpop_grouped::grouped_pipeline(lists, &client, lpop_list).await
    };
    lists.perform(admission, call, None, request).await
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
        if let Value::Array(items) = value {
            entries.push(decode_group(&popped_items(items, "async_lpop_pipeline")?));
            continue;
        }
        let Some(bytes) = reply_bytes(value, "async_lpop_pipeline")? else {
            entries.push(LpopPipelineEntry::Missing);
            continue;
        };
        entries.push(match str::from_utf8(bytes) {
            Ok(text) => LpopPipelineEntry::Items(vec![text.to_owned()]),
            Err(_source) => LpopPipelineEntry::ItemsBytes(vec![bytes.to_vec()]),
        });
    }
    Ok(entries)
}
