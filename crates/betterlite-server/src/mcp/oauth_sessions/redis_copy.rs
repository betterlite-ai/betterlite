use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::registry::resolution::ServerAccessCheck;
use betterlite_mcp::server_record::MCPServer;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue, StoredValue};
use num_traits::ToPrimitive as _;
use secrecy::{ExposeSecret as _, SecretString};

use super::store::SessionStore;
// ast-grep-ignore: no-long-names
use super::TEMPORARY_MCP_SERVER_REDIS_KEY_PREFIX as REDIS_PREFIX;

mod tests;

// the field label LiteLLM passes to decrypt_value_helper for the temporary server payload
const REDIS_FIELD: &str = "temporary_mcp_server";

// LiteLLM's _cache_temporary_mcp_server_in_redis: best effort, so every failure is one debug line
pub async fn write_redis_copy<Access>(
    store: &SessionStore<Access>,
    server: &MCPServer,
    ttl_seconds: i64,
) where
    Access: ServerAccessCheck,
{
    let Some(redis) = store.redis.as_ref() else {
        return;
    };
    let encoded = match serde_json::to_string(server) {
        Ok(encoded) => encoded,
        Err(error) => {
            tracing::debug!(
                "Failed to encode temporary MCP server payload for Redis cache: {error}"
            );
            return;
        }
    };
    let sealed = match encrypt_redis_copy(store.cipher.as_ref(), encoded) {
        Ok(sealed) => sealed,
        Err(error) => {
            tracing::debug!(
                "Failed to encrypt temporary MCP server payload for Redis cache: {error}"
            );
            return;
        }
    };
    let ttl = ttl_seconds.max(1).to_f64().unwrap_or(f64::MAX);
    let server_id = server.fields.server_id.as_str();
    let key = format!("{REDIS_PREFIX}:{server_id}");
    if let Err(error) = redis
        .async_set_cache(&key, CacheValue::Text(sealed), Some(ttl))
        .await
    {
        tracing::debug!("Failed to write temporary MCP server to Redis cache: {error}");
    }
}

// LiteLLM's _get_temporary_mcp_server_from_redis: encrypted strings only, every failure gives None
pub async fn read_redis_copy<Access>(
    store: &SessionStore<Access>,
    server_id: &str,
) -> Option<MCPServer>
where
    Access: ServerAccessCheck,
{
    let redis = store.redis.as_ref()?;
    let key = format!("{REDIS_PREFIX}:{server_id}");
    let cached = match redis.async_get_cache(&key).await {
        Ok(cached) => cached,
        Err(error) => {
            tracing::debug!("Failed reading temporary MCP server from Redis cache: {error}");
            return None;
        }
    };
    let Some(cached) = cached else {
        return None;
    };
    let CacheValue::Text(sealed) = cached else {
        tracing::debug!(
            "Temporary MCP Redis cache value must be an encrypted string; rejecting non-string payload"
        );
        return None;
    };
    decode_redis_copy(store.cipher.as_ref(), &sealed)
}

// LiteLLM loads the decrypted text as a dict and then as MCPServer; either failure is a miss
fn decode_redis_copy(cipher: &DbCipher, sealed: &str) -> Option<MCPServer> {
    let decrypted = decrypt_redis_copy(cipher, sealed)?;
    let parsed = match serde_json::from_str::<JsonData>(&decrypted) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::debug!("Invalid decrypted temporary MCP payload in Redis cache: {error}");
            return None;
        }
    };
    let JsonData::Object(_payload) = parsed else {
        tracing::debug!("Invalid temporary MCP server payload in Redis cache: not an object");
        return None;
    };
    match serde_json::from_str::<MCPServer>(&decrypted) {
        Ok(server) => Some(server),
        Err(error) => {
            tracing::debug!("Invalid temporary MCP server payload in Redis cache: {error}");
            None
        }
    }
}

fn encrypt_redis_copy(cipher: &DbCipher, plaintext: String) -> Result<String, SecretError> {
    let stored = StoredValue::<String>::Text(SecretString::from(plaintext));
    match cipher.encrypt_value(stored, None)? {
        StoredValue::Text(sealed) => Ok(sealed.expose_secret().to_owned()),
        StoredValue::Other(value) => Ok(value),
    }
}

// LiteLLM's decrypt_value_helper with the debug error mode and return_original_value=False
fn decrypt_redis_copy(cipher: &DbCipher, sealed: &str) -> Option<String> {
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    let stored = StoredValue::<String>::Text(SecretString::from(sealed.to_owned()));
    match cipher.decrypt_value(stored, REDIS_FIELD, policy)? {
        StoredValue::Text(plaintext) => Some(plaintext.expose_secret().to_owned()),
        StoredValue::Other(value) => Some(value),
    }
}
