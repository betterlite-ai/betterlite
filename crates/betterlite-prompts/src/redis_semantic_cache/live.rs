use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use redis::Value;
use redis::cmd;
use secrecy::SecretString;

use betterlite_redis::redis_client_factories::RedisClient;
use betterlite_redis::redis_client_factories::url::UrlClient;
use betterlite_redis::redis_live_client::{LiveAsyncClient, LiveClientError, LiveClientFactory};

use super::error::RedisSemanticError;
use super::ports::{
    CacheEntryFields, Hit, IndexSchema, SemanticIndexConnector, SemanticRedisIndex,
    SemanticSearchQuery,
};
use super::query::parse_search_reply;
use crate::arguments::json::Json;

const MISSING_INDEX_FRAGMENTS: [&str; 4] = [
    "unknown index name",
    "no such index",
    "search_index_not_found",
    "index not found",
];

pub(crate) struct LiveSemanticIndex {
    client: LiveAsyncClient,
}

pub struct LiveSemanticIndexConnector;

#[async_trait]
impl SemanticRedisIndex for LiveSemanticIndex {
    async fn ft_info(&self, index: &str) -> Result<Option<Json>, RedisSemanticError> {
        let mut command = cmd("FT.INFO");
        command.arg(index);
        match self.client.query::<Value>(&command).await {
            Ok(reply) => Ok(Some(value_to_json(&reply)?)),
            Err(LiveClientError::Redis(error)) if is_missing_index_error(&error) => Ok(None),
            Err(error) => Err(RedisSemanticError::Backend(error)),
        }
    }

    async fn ft_create(&self, schema: &IndexSchema) -> Result<(), RedisSemanticError> {
        self.client.query::<Value>(&create_command(schema)).await?;
        Ok(())
    }

    async fn ft_dropindex(&self, index: &str) -> Result<(), RedisSemanticError> {
        let mut command = cmd("FT.DROPINDEX");
        command.arg(index);
        self.client.query::<Value>(&command).await?;
        Ok(())
    }

    async fn ft_search(
        &self,
        index: &str,
        query: &SemanticSearchQuery,
    ) -> Result<Vec<Hit>, RedisSemanticError> {
        let reply: Value = self.client.query(&search_command(index, query)).await?;
        parse_search_reply(&value_to_json(&reply)?)
    }

    async fn store_hash(
        &self,
        key: &str,
        fields: &CacheEntryFields,
        ttl: Option<i64>,
    ) -> Result<(), RedisSemanticError> {
        self.client
            .query::<Value>(&store_command(key, fields))
            .await?;
        if let Some(seconds) = ttl {
            let mut command = cmd("EXPIRE");
            command.arg(key).arg(seconds);
            self.client.query::<Value>(&command).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl SemanticIndexConnector for LiveSemanticIndexConnector {
    async fn connect(
        &self,
        redis_url: &str,
    ) -> Result<Arc<dyn SemanticRedisIndex>, RedisSemanticError> {
        let record = UrlClient {
            url: SecretString::from(redis_url.to_owned()),
            username: None,
            socket_timeout: None,
            socket_connect_timeout: None,
            socket_keepalive: None,
            max_connections: None,
            health_check_interval: None,
            credential_provider: false,
            credential_provider_settings: None,
            passthrough: BTreeMap::new(),
        };
        let client = LiveClientFactory::new(None)
            .open_async(&RedisClient::Url(record), None)
            .await?;
        Ok(Arc::new(LiveSemanticIndex { client }))
    }
}

const CREATE_SCHEMA: [&str; 10] = [
    "SCHEMA",
    "prompt",
    "TEXT",
    "response",
    "TEXT",
    "inserted_at",
    "NUMERIC",
    "updated_at",
    "NUMERIC",
    "prompt_vector",
];

const CREATE_VECTOR: [&str; 6] = ["VECTOR", "FLAT", "6", "TYPE", "FLOAT32", "DIM"];

const RETURN_FIELDS: [&str; 8] = [
    "entry_id",
    "prompt",
    "response",
    "inserted_at",
    "updated_at",
    "metadata",
    "litellm_cache_key",
    "vector_distance",
];

pub(crate) fn create_command(schema: &IndexSchema) -> redis::Cmd {
    let mut command = cmd("FT.CREATE");
    command.arg(&schema.name).arg(&["ON", "HASH", "PREFIX"]);
    command.arg(1_usize).arg(schema.prefix.as_str());
    command.arg(&CREATE_SCHEMA);
    command.arg(&CREATE_VECTOR).arg(schema.dimension);
    command.arg(&["DISTANCE_METRIC", "COSINE", "litellm_cache_key", "TAG"]);
    command
}

pub(crate) fn search_command(index: &str, query: &SemanticSearchQuery) -> redis::Cmd {
    let vector: Vec<u8> = query
        .vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let threshold = query.distance_threshold.to_string();
    let mut command = cmd("FT.SEARCH");
    command.arg(index).arg(query.query_string());
    command.arg(&["PARAMS", "4", "vector"]).arg(vector);
    command.arg(&["distance_threshold", threshold.as_str()]);
    command
        .arg("RETURN")
        .arg(RETURN_FIELDS.len())
        .arg(&RETURN_FIELDS);
    command.arg(&["SORTBY", "vector_distance", "ASC"]);
    command.arg(&["LIMIT", "0", "1", "DIALECT", "2"]);
    command
}

pub(crate) fn store_command(key: &str, fields: &CacheEntryFields) -> redis::Cmd {
    let inserted_at = fields.inserted_at.to_string();
    let updated_at = fields.updated_at.to_string();
    let mut command = cmd("HSET");
    command.arg(key);
    command.arg(&["entry_id", fields.entry_id.as_str()]);
    command.arg(&["prompt", fields.prompt.as_str()]);
    command.arg(&["response", fields.response.as_str()]);
    command.arg("prompt_vector");
    command.arg(fields.prompt_vector.as_slice());
    command.arg(&["inserted_at", inserted_at.as_str()]);
    command.arg(&["updated_at", updated_at.as_str()]);
    command.arg(&["litellm_cache_key", fields.litellm_cache_key.as_str()]);
    command
}

pub(crate) fn is_missing_index_error(error: &redis::RedisError) -> bool {
    // redis-rs splits the first word of a server error into the code, so rebuild the text.
    let message = [error.code(), error.detail()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    MISSING_INDEX_FRAGMENTS
        .iter()
        .any(|fragment| message.contains(fragment))
}

fn value_to_json(value: &Value) -> Result<Json, RedisSemanticError> {
    match value {
        Value::Nil => Ok(Json::Null),
        Value::Int(number) => Ok(Json::Number(serde_json::Number::from(*number))),
        Value::BulkString(bytes) => Ok(Json::Text(String::from_utf8(bytes.clone())?)),
        Value::SimpleString(text) => Ok(Json::Text(text.clone())),
        Value::Array(items) => Ok(Json::List(
            items.iter().map(value_to_json).collect::<Result<_, _>>()?,
        )),
        Value::Okay
        | Value::Map(_)
        | Value::Attribute { .. }
        | Value::Set(_)
        | Value::Double(_)
        | Value::Boolean(_)
        | Value::VerbatimString { .. }
        | Value::BigNumber(_)
        | Value::Push { .. }
        | Value::ServerError(_) => Err(RedisSemanticError::UnexpectedReply { kind: "value" }),
        // redis::Value is non_exhaustive; a variant added later lands here.
        _ => Err(RedisSemanticError::UnexpectedReply {
            kind: "unknown value",
        }),
    }
}
