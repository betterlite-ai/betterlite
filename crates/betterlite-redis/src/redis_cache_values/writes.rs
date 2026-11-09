use std::time::Instant;

use redis::Cmd;
use serde::Serialize;

use super::RedisCacheValues;
use super::error::RedisCacheValuesError;
use super::events::{event, finish_outcome};
use super::naming::apply_namespace;
use super::ttl::resolve_ttl;
use crate::cache_primitives::protocol::AsyncRedisCommands as _;
use crate::redis_client_reuse::breaker::BreakerAdmission;
use crate::redis_live_client::LiveAsyncClient;

pub(crate) fn set_command<V>(
    default_ttl: Option<i64>,
    key: &str,
    value: &V,
    ttl: Option<f64>,
    nx: bool,
) -> Result<Cmd, RedisCacheValuesError>
where
    V: Serialize + ?Sized,
{
    let text = serde_json::to_string(value)?;
    let seconds = resolve_ttl(ttl, default_ttl)?;
    let mut command = redis::cmd("SET");
    command.arg(key).arg(text);
    if nx {
        command.arg("NX");
    }
    if let Some(expiry) = seconds {
        command.arg("EX").arg(expiry);
    }
    Ok(command)
}

pub(crate) async fn execute_pipeline(
    client: &LiveAsyncClient,
    commands: Vec<Cmd>,
) -> Result<(), RedisCacheValuesError> {
    let mut pipeline = client.pipeline(false);
    for command in commands {
        pipeline.commands.add_command(command);
    }
    let _results = pipeline.execute().await?;
    Ok(())
}

pub(crate) async fn write_sadd(
    client: &LiveAsyncClient,
    key: &str,
    members: &[String],
    ttl: Option<f64>,
    default_ttl: Option<i64>,
) -> Result<(), RedisCacheValuesError> {
    let seconds = resolve_ttl(ttl, default_ttl)?;
    let mut command = redis::cmd("SADD");
    command.arg(key);
    for member in members {
        command.arg(member);
    }
    let _added: i64 = client.query(&command).await?;
    if let Some(expiry) = seconds {
        let mut expire = redis::cmd("EXPIRE");
        expire.arg(key).arg(expiry);
        let _expired: bool = client.query(&expire).await?;
    }
    Ok(())
}

pub(crate) async fn send_ttls_pipeline(
    values: &RedisCacheValues,
    built: Result<Vec<Cmd>, RedisCacheValuesError>,
    admission: BreakerAdmission,
    started: Instant,
) -> Result<(), RedisCacheValuesError> {
    let outcome = match built {
        Ok(commands) => match values.transport.async_client().await {
            Ok(client) => execute_pipeline(&client, commands).await,
            Err(source) => Err(RedisCacheValuesError::ClientReuse(source)),
        },
        // a finite TTL beyond i64 raises inside LiteLLM's timedelta() and is swallowed
        Err(RedisCacheValuesError::TtlOutOfRange { seconds }) if seconds.is_finite() => {
            Err(RedisCacheValuesError::TtlOutOfRange { seconds })
        }
        Err(error) => {
            values.gate.record_failure(&error);
            return Err(error);
        }
    };
    let finished = event("async_set_cache_pipeline_with_ttls", started, None);
    let _stored = finish_outcome(values, &finished, admission, outcome);
    Ok(())
}

pub(crate) async fn send_shared_pipeline<V>(
    values: &RedisCacheValues,
    client: &LiveAsyncClient,
    cache_list: &[(String, V)],
    ttl: Option<f64>,
) -> Result<(), RedisCacheValuesError>
where
    V: Serialize + Sync,
{
    let built: Result<Vec<Cmd>, RedisCacheValuesError> = cache_list
        .iter()
        .map(|(key, value)| {
            let namespaced = apply_namespace(values, key);
            set_command(values.settings.default_ttl, &namespaced, value, ttl, false)
        })
        .collect();
    execute_pipeline(client, built?).await
}
