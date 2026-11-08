use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_redis::redis_client_factories::RedisClient;
use betterlite_redis::redis_client_factories::url::UrlClient;
use betterlite_redis::redis_live_client::{LiveAsyncClient, LiveClientFactory};
use redis::Value;
use redis::cmd;
use secrecy::SecretString;

use super::error::ValkeySemanticError;
use super::ports::{DocMapping, Hit, IndexSchema, KnnQuery, ValkeyConnector, ValkeyIndex};
use super::schema::{
    DISTANCE_FIELD_NAME, EMBEDDING_FIELD_NAME, PROMPT_FIELD_NAME, RESPONSE_FIELD_NAME,
    VECTOR_PARAM_NAME,
};
use crate::arguments::json::{Json, Object};
use crate::redis_semantic_cache::CACHE_KEY_FIELD_NAME;

pub struct LiveValkeyConnector;

pub struct LiveValkeyIndex {
    client: LiveAsyncClient,
}

#[async_trait]
impl ValkeyConnector for LiveValkeyConnector {
    async fn connect(
        &self,
        url: &SecretString,
    ) -> Result<Arc<dyn ValkeyIndex>, ValkeySemanticError> {
        let record = UrlClient {
            url: url.clone(),
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
        Ok(Arc::new(LiveValkeyIndex { client }))
    }
}

#[async_trait]
impl ValkeyIndex for LiveValkeyIndex {
    async fn ft_create(
        &self,
        index: &str,
        schema: &IndexSchema,
    ) -> Result<(), ValkeySemanticError> {
        self.client
            .query::<Value>(&create_command(index, schema))
            .await?;
        Ok(())
    }

    async fn ft_info(&self, index: &str) -> Result<Json, ValkeySemanticError> {
        let mut command = cmd("FT.INFO");
        command.arg(index);
        let reply: Vec<Value> = self.client.query(&command).await?;
        info_object(&reply)
    }

    async fn ft_search(
        &self,
        index: &str,
        query: &KnnQuery,
        vector: &[u8],
    ) -> Result<Vec<Hit>, ValkeySemanticError> {
        let reply: Vec<Value> = self
            .client
            .query(&search_command(index, query, vector))
            .await?;
        parse_search_reply(&reply)
    }

    async fn store_hash(
        &self,
        key: &str,
        fields: &DocMapping,
        ttl: Option<i64>,
    ) -> Result<(), ValkeySemanticError> {
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

pub(crate) fn create_command(index: &str, schema: &IndexSchema) -> redis::Cmd {
    let mut command = cmd("FT.CREATE");
    command
        .arg(index)
        .arg(&["ON", "HASH", "PREFIX"])
        .arg(1_usize)
        .arg(schema.prefix.as_str());
    command
        .arg("SCHEMA")
        .arg(CACHE_KEY_FIELD_NAME)
        .arg(&["TAG", "SEPARATOR", ","]);
    command
        .arg(EMBEDDING_FIELD_NAME)
        .arg(&["VECTOR", "HNSW", "6", "TYPE", "FLOAT32", "DIM"]);
    command
        .arg(schema.dimension)
        .arg(&["DISTANCE_METRIC", "COSINE"]);
    command
}

pub(crate) fn search_command(index: &str, query: &KnnQuery, vector: &[u8]) -> redis::Cmd {
    let mut command = cmd("FT.SEARCH");
    command.arg(index).arg(query.text.as_str());
    command.arg("RETURN").arg(query.return_fields.len());
    for field in &query.return_fields {
        command.arg(field);
    }
    command.arg("DIALECT").arg(query.dialect);
    // redis-py's Query carries its default paging on every search, and LiteLLM never changes it
    command.arg("LIMIT").arg(0_usize).arg(10_usize);
    command
        .arg("PARAMS")
        .arg(2_usize)
        .arg(VECTOR_PARAM_NAME)
        .arg(vector);
    command
}

pub(crate) fn store_command(key: &str, fields: &DocMapping) -> redis::Cmd {
    let mut command = cmd("HSET");
    command.arg(key);
    command
        .arg(CACHE_KEY_FIELD_NAME)
        .arg(fields.litellm_cache_key.as_str());
    command.arg(PROMPT_FIELD_NAME).arg(fields.prompt.as_str());
    command
        .arg(RESPONSE_FIELD_NAME)
        .arg(fields.response.as_str());
    command
        .arg(EMBEDDING_FIELD_NAME)
        .arg(fields.embedding.as_slice());
    command
}

fn info_object(items: &[Value]) -> Result<Json, ValkeySemanticError> {
    let (pairs, remainder) = items.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(ValkeySemanticError::MalformedIndexInfo);
    }
    let mut object = Object::new();
    for [name, value] in pairs {
        let Value::BulkString(raw_name) = name else {
            return Err(ValkeySemanticError::MalformedIndexInfo);
        };
        object.insert(String::from_utf8(raw_name.clone())?, reply_to_json(value)?);
    }
    Ok(Json::Object(object))
}

fn reply_to_json(value: &Value) -> Result<Json, ValkeySemanticError> {
    match value {
        Value::Int(number) => Ok(Json::Number(serde_json::Number::from(*number))),
        Value::BulkString(bytes) => Ok(Json::Text(String::from_utf8(bytes.clone())?)),
        Value::SimpleString(text) => Ok(Json::Text(text.clone())),
        Value::Nil => Ok(Json::Null),
        Value::Array(items) => {
            let mut converted = Vec::with_capacity(items.len());
            for item in items {
                converted.push(reply_to_json(item)?);
            }
            Ok(Json::List(converted))
        }
        // redis::Value is non_exhaustive, so the wildcard catches a variant added later
        Value::Okay
        | Value::Map(_)
        | Value::Attribute { .. }
        | Value::Set(_)
        | Value::Double(_)
        | Value::Boolean(_)
        | Value::VerbatimString { .. }
        | Value::BigNumber(_)
        | Value::Push { .. }
        | Value::ServerError(_)
        | _ => Err(ValkeySemanticError::UnexpectedReply { kind: "reply" }),
    }
}

fn parse_search_reply(items: &[Value]) -> Result<Vec<Hit>, ValkeySemanticError> {
    let Some(Value::Int(_)) = items.first() else {
        return Err(ValkeySemanticError::MalformedSearchReply);
    };
    let (documents, remainder) = items.get(1..).unwrap_or_default().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(ValkeySemanticError::MalformedSearchReply);
    }
    let mut hits = Vec::with_capacity(documents.len());
    for [_key, document] in documents {
        let Value::Array(fields) = document else {
            return Err(ValkeySemanticError::MalformedSearchReply);
        };
        hits.push(hit_of(fields)?);
    }
    Ok(hits)
}

fn hit_of(fields: &[Value]) -> Result<Hit, ValkeySemanticError> {
    let (pairs, remainder) = fields.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(ValkeySemanticError::MalformedSearchReply);
    }
    let mut response_text = None;
    let mut distance_value = None;
    for [name, value] in pairs {
        match text_of(name)?.as_str() {
            RESPONSE_FIELD_NAME => response_text = Some(text_of(value)?),
            DISTANCE_FIELD_NAME => distance_value = Some(distance_of(value)?),
            _ => {}
        }
    }
    let (Some(response), Some(distance)) = (response_text, distance_value) else {
        return Err(ValkeySemanticError::MalformedSearchReply);
    };
    Ok(Hit { response, distance })
}

fn text_of(value: &Value) -> Result<String, ValkeySemanticError> {
    match reply_to_json(value)? {
        Json::Text(text) => Ok(text),
        Json::Number(number) => Ok(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => {
            Err(ValkeySemanticError::MalformedSearchReply)
        }
    }
}

fn distance_of(value: &Value) -> Result<f64, ValkeySemanticError> {
    if let Value::Double(number) = value {
        return Ok(*number);
    }
    text_of(value)?
        .parse()
        .map_err(|_error| ValkeySemanticError::MalformedSearchReply)
}
