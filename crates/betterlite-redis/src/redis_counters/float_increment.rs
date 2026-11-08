use std::str;

use redis::{Value, cmd};

use crate::cache_primitives::float_ttl::{TtlFollowup, needs_standalone_expire, plan_ttl_followup};
use crate::cache_primitives::pipeline_increment::PipelineStep;
use crate::cache_primitives::pipeline_increment::PipelineStep::IncrByFloat;
use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::redis_live_client::LiveAsyncClient;
use crate::redis_live_client::pipeline::LivePipeline;

use super::RedisCountersError;
use super::reply::{RedisReply, classify};

pub(crate) async fn increment_with_followup(
    client: &LiveAsyncClient,
    key: &str,
    value: f64,
    ttl_seconds: i64,
    refresh_ttl: bool,
) -> Result<f64, RedisCountersError> {
    let unexpected = || RedisCountersError::UnexpectedReply {
        operation: "incr_by_float_with_ttl",
    };
    let (pipeline, followup) = ttl_pipeline(client, key, value, ttl_seconds, refresh_ttl);
    let replies = pipeline.execute().await?;
    let Some(increment_reply) = replies.first() else {
        return Err(unexpected());
    };
    let result = float_reply(increment_reply.clone(), "incr_by_float_with_ttl")?;
    if followup == Some(TtlFollowup::Ttl) {
        let Some(Value::Int(answered)) = replies.get(1) else {
            return Err(unexpected());
        };
        if needs_standalone_expire(followup, *answered) {
            client.expire(key, ttl_seconds).await?;
        }
    }
    Ok(result)
}

fn ttl_pipeline(
    client: &LiveAsyncClient,
    key: &str,
    value: f64,
    ttl_seconds: i64,
    refresh_ttl: bool,
) -> (LivePipeline, Option<TtlFollowup>) {
    let mut pipeline = client.pipeline(false);
    add_pipeline_step(
        &mut pipeline,
        &IncrByFloat {
            key: key.to_owned(),
            value,
        },
    );
    let followup = plan_ttl_followup(Some(ttl_seconds), refresh_ttl);
    match followup {
        Some(TtlFollowup::Expire) => add_pipeline_step(
            &mut pipeline,
            &PipelineStep::Expire {
                key: key.to_owned(),
                ttl: ttl_seconds,
            },
        ),
        Some(TtlFollowup::Ttl) => {
            let mut ask_ttl = cmd("TTL");
            ask_ttl.arg(key);
            pipeline.commands.add_command(ask_ttl);
        }
        None => {}
    }
    (pipeline, followup)
}

pub(crate) fn add_pipeline_step(pipeline: &mut LivePipeline, step: &PipelineStep) {
    let command = match step {
        IncrByFloat { key, value } => {
            let mut command = cmd("INCRBYFLOAT");
            command.arg(key).arg(*value);
            command
        }
        PipelineStep::Expire { key, ttl } => {
            let mut command = cmd("EXPIRE");
            command.arg(key).arg(*ttl);
            command
        }
    };
    pipeline.commands.add_command(command);
}

pub(crate) fn float_reply(
    reply: Value,
    operation: &'static str,
) -> Result<f64, RedisCountersError> {
    let unexpected = || RedisCountersError::UnexpectedReply { operation };
    match classify(reply) {
        RedisReply::Bulk(bytes) => {
            let Ok(text) = str::from_utf8(&bytes) else {
                return Err(unexpected());
            };
            text.parse().map_err(|_source| unexpected())
        }
        RedisReply::Double(value) => Ok(value),
        RedisReply::Nil | RedisReply::Int(_) | RedisReply::Array(_) | RedisReply::Unsupported => {
            Err(unexpected())
        }
    }
}
