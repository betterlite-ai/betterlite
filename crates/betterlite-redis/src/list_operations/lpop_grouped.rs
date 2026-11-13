use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::redis_live_client::LiveAsyncClient;

use super::error::RedisListOperationsError;
use super::lpop::lpop_command;
use super::results::{LpopPipelineEntry, decode_group, reply_bytes};
use super::{RedisListOperations, RedisPipelineLpopOperation, namespaced_key};

pub(crate) async fn grouped_pipeline(
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
    for (key, count) in &planned {
        for _ in 0..*count {
            pipeline.commands.add_command(lpop_command(key, None));
        }
    }
    let flat = pipeline.execute().await?;
    let mut values = flat.into_iter();
    let mut entries = Vec::with_capacity(planned.len());
    for (_, count) in &planned {
        let mut group = Vec::new();
        for _ in 0..*count {
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
