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
use crate::arguments::json::{Json, Object};

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

pub(crate) fn create_command(schema: &IndexSchema) -> redis::Cmd {
    let mut command = cmd("FT.CREATE");
    command
        .arg(&schema.name)
        .arg("ON")
        .arg("HASH")
        .arg("PREFIX")
        .arg(1_usize)
        .arg(&schema.prefix)
        .arg("SCHEMA")
        .arg("prompt")
        .arg("TEXT")
        .arg("response")
        .arg("TEXT")
        .arg("inserted_at")
        .arg("NUMERIC")
        .arg("updated_at")
        .arg("NUMERIC")
        .arg("prompt_vector")
        .arg("VECTOR")
        .arg("FLAT")
        .arg(6_usize)
        .arg("TYPE")
        .arg("FLOAT32")
        .arg("DIM")
        .arg(schema.dimension)
        .arg("DISTANCE_METRIC")
        .arg("COSINE")
        .arg("litellm_cache_key")
        .arg("TAG");
    command
}

pub(crate) fn search_command(index: &str, query: &SemanticSearchQuery) -> redis::Cmd {
    let vector: Vec<u8> = query
        .vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let mut command = cmd("FT.SEARCH");
    command
        .arg(index)
        .arg(query.query_string())
        .arg("PARAMS")
        .arg(4_usize)
        .arg("vector")
        .arg(vector)
        .arg("distance_threshold")
        .arg(query.distance_threshold)
        .arg("RETURN")
        .arg(7_usize)
        .arg("entry_id")
        .arg("prompt")
        .arg("response")
        .arg("inserted_at")
        .arg("updated_at")
        .arg("metadata")
        .arg("litellm_cache_key")
        .arg("vector_distance")
        .arg("SORTBY")
        .arg("vector_distance")
        .arg("ASC")
        .arg("LIMIT")
        .arg(0_usize)
        .arg(1_usize)
        .arg("DIALECT")
        .arg(2_usize);
    command
}

pub(crate) fn store_command(key: &str, fields: &CacheEntryFields) -> redis::Cmd {
    let mut command = cmd("HSET");
    command
        .arg(key)
        .arg("entry_id")
        .arg(&fields.entry_id)
        .arg("prompt")
        .arg(&fields.prompt)
        .arg("response")
        .arg(&fields.response)
        .arg("prompt_vector")
        .arg(fields.prompt_vector.as_slice())
        .arg("inserted_at")
        .arg(fields.inserted_at.to_string())
        .arg("updated_at")
        .arg(fields.updated_at.to_string())
        .arg("litellm_cache_key")
        .arg(&fields.litellm_cache_key);
    command
}

fn is_missing_index_error(error: &redis::RedisError) -> bool {
    let message = error.to_string().to_lowercase();
    MISSING_INDEX_FRAGMENTS
        .iter()
        .any(|fragment| message.contains(fragment))
}

fn map_to_object(pairs: &[(Value, Value)]) -> Result<Object, RedisSemanticError> {
    let mut object = Object::new();
    for (key, item) in pairs {
        let Json::Text(name) = value_to_json(key)? else {
            return Err(RedisSemanticError::UnexpectedReply { kind: "map key" });
        };
        object.insert(name, value_to_json(item)?);
    }
    Ok(object)
}

fn value_to_json(value: &Value) -> Result<Json, RedisSemanticError> {
    match value {
        Value::Nil => Ok(Json::Null),
        Value::Int(number) => Ok(Json::Number(serde_json::Number::from(*number))),
        Value::BulkString(bytes) => Ok(Json::Text(String::from_utf8(bytes.clone())?)),
        Value::SimpleString(text) => Ok(Json::Text(text.clone())),
        Value::Okay => Ok(Json::Text("OK".to_owned())),
        Value::Array(items) => {
            let converted = items
                .iter()
                .map(value_to_json)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Json::List(converted))
        }
        Value::Map(pairs) => Ok(Json::Object(map_to_object(pairs)?)),
        Value::Double(number) => serde_json::Number::from_f64(*number)
            .map(Json::Number)
            .ok_or(RedisSemanticError::UnexpectedReply {
                kind: "non-finite double",
            }),
        Value::Boolean(flag) => Ok(Json::Bool(*flag)),
        Value::Attribute { .. }
        | Value::Set(_)
        | Value::VerbatimString { .. }
        | Value::BigNumber(_)
        | Value::Push { .. }
        | Value::ServerError(_) => Err(RedisSemanticError::UnexpectedReply {
            kind: "unsupported value",
        }),
        // redis::Value is non_exhaustive, so a variant added later lands here.
        _ => Err(RedisSemanticError::UnexpectedReply {
            kind: "unknown value",
        }),
    }
}
