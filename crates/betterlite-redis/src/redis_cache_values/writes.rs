use redis::Cmd;
use serde::Serialize;

use super::RedisCacheValues;
use super::error::RedisCacheValuesError;
use super::naming::apply_namespace;
use super::ttl::resolve_ttl;
use crate::cache_primitives::protocol::AsyncRedisCommands as _;
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

pub(crate) fn one_ttl_commands<V>(
    values: &RedisCacheValues,
    cache_list: &[(String, V)],
    ttl: Option<f64>,
) -> Result<Vec<Cmd>, RedisCacheValuesError>
where
    V: Serialize,
{
    cache_list
        .iter()
        .map(|(key, value)| {
            let namespaced = apply_namespace(values, key);
            set_command(values.default_ttl, &namespaced, value, ttl, false)
        })
        .collect()
}

pub(crate) fn per_entry_commands<V>(
    values: &RedisCacheValues,
    cache_list: &[(String, V, Option<f64>)],
) -> Result<Vec<Cmd>, RedisCacheValuesError>
where
    V: Serialize,
{
    cache_list
        .iter()
        .map(|(key, value, ttl)| {
            let namespaced = apply_namespace(values, key);
            set_command(values.default_ttl, &namespaced, value, *ttl, false)
        })
        .collect()
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
