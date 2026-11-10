use redis::cmd;

use super::error::RedisCacheError;
use crate::cache_primitives::decode::{CachedValue, decode_cached_value};
use crate::redis_live_client::LiveAsyncClient;

pub(crate) async fn get_cached(
    client: &LiveAsyncClient,
    key: &str,
) -> Result<Option<CachedValue>, RedisCacheError> {
    let mut command = cmd("GET");
    command.arg(key);
    let reply: Option<Vec<u8>> = client.query(&command).await?;
    Ok(decode_cached_value(reply.as_deref())?)
}
