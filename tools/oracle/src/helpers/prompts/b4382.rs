use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json, to_value};

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message};
use betterlite_prompts::redis_semantic_cache::error::RedisSemanticError;
use betterlite_prompts::redis_semantic_cache::options::{RedisEnv, RedisSemanticOptions};
use betterlite_prompts::redis_semantic_cache::ports::{
    CacheEntryFields, Hit, IndexSchema, SemanticEmbedError, SemanticEmbedder,
    SemanticIndexConnector, SemanticRedisIndex, SemanticSearchQuery,
};
use betterlite_prompts::redis_semantic_cache::{
    CACHE_KEY_FIELD_NAME, RedisSemanticCache, SIMILARITY_FIELD, SemanticCacheKwargs,
};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Constructor(ConstructorInput),
    KeyScope(KeyScopeInput),
    IndexFallback(IndexFallbackInput),
    Lookup(LookupInput),
    Store(StoreInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConstructorInput {
    #[serde(default)]
    threshold: Option<f64>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    redis_url: Option<String>,
    #[serde(default)]
    index_name: Option<String>,
    #[serde(default)]
    embedding_model: Option<String>,
    #[serde(default)]
    embedding_max_input_tokens: Option<i64>,
    embedding_timeout: f64,
    #[serde(default)]
    env: ConstructorEnv,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConstructorEnv {
    #[serde(default)]
    redis_host: Option<String>,
    #[serde(default)]
    redis_port: Option<String>,
    #[serde(default)]
    redis_password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyScopeInput {
    key: String,
    #[serde(default)]
    litellm_cache_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IndexFallbackInput {
    index_name: String,
    dimension: usize,
    #[serde(default)]
    mismatch_attempts: Vec<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LookupInput {
    key: String,
    #[serde(default)]
    messages: Option<Vec<Message>>,
    #[serde(default, rename = "input")]
    responses_input: Option<Json>,
    #[serde(default)]
    metadata: Option<Json>,
    vector: Vec<f32>,
    #[serde(default)]
    hits: Vec<HitInput>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HitInput {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    response: Option<String>,
    #[serde(default)]
    litellm_cache_key: Option<String>,
    #[serde(default)]
    vector_distance: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoreInput {
    key: String,
    #[serde(default)]
    messages: Option<Vec<Message>>,
    #[serde(default, rename = "input")]
    responses_input: Option<Json>,
    #[serde(default)]
    metadata: Option<Json>,
    #[serde(default)]
    ttl: Option<i64>,
    vector: Vec<f32>,
    value: Json,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(|error| helper_error("RuntimeError", error))?;
        runtime.block_on(dispatch(input))
    }
}

async fn dispatch(input: &Input) -> HelperResult {
    match input {
        Input::Constructor(settings) => constructor(settings),
        Input::KeyScope(scope) => Ok(key_scope(scope)),
        Input::IndexFallback(fallback) => index_fallback(fallback).await,
        Input::Lookup(lookup) => run_lookup(lookup).await,
        Input::Store(store) => run_store(store).await,
    }
}

fn constructor(input: &ConstructorInput) -> HelperResult {
    let options = RedisSemanticOptions {
        host: input.host.clone(),
        port: input.port.clone(),
        password: input.password.clone(),
        redis_url: input.redis_url.clone(),
        similarity_threshold: input.threshold,
        embedding_model: input.embedding_model.clone(),
        index_name: input.index_name.clone(),
        embedding_max_input_tokens: input.embedding_max_input_tokens,
        embedding_timeout: input.embedding_timeout,
    };
    let env = RedisEnv {
        redis_host: input.env.redis_host.clone(),
        redis_port: input.env.redis_port.clone(),
        redis_password: input.env.redis_password.clone(),
    };
    let cache = RedisSemanticCache::new(
        options,
        env,
        Arc::new(UnusedEmbedder),
        Arc::new(UnusedConnector),
    )
    .map_err(|error| helper_error("ValueError", error))?;
    Ok(json!({
        "index_name": cache.index_name,
        "distance_threshold": cache.distance_threshold,
        "redis_url": cache.redis_url.expose_secret(),
        "embedding_model": cache.embedding_model,
        "embedding_max_input_tokens": cache.embedding_max_input_tokens,
        "embedding_timeout": cache.embedding_timeout,
    }))
}

fn key_scope(input: &KeyScopeInput) -> Value {
    let candidate = Hit {
        entry_id: None,
        prompt: None,
        response: None,
        inserted_at: None,
        updated_at: None,
        metadata: None,
        litellm_cache_key: input.litellm_cache_key.clone(),
        vector_distance: None,
    };
    json!({ "matches": RedisSemanticCache::cache_hit_matches_key(&candidate, &input.key) })
}

async fn index_fallback(input: &IndexFallbackInput) -> HelperResult {
    let index = Arc::new(ScriptedIndex {
        dimension: input.dimension,
        mismatch_attempts: input.mismatch_attempts.clone(),
        info_calls: Mutex::new(0),
        ops: Mutex::new(Vec::new()),
    });
    let cache = helper_cache(
        Arc::new(ConstantEmbedder {
            vector: vec![0.5; input.dimension],
        }),
        index.clone(),
    )?;
    let messages = [user_message("fallback")];
    let kwargs = helper_kwargs(Some(&messages), None, None, None);
    cache.async_set_cache("scope-1", &Json::Null, &kwargs).await;
    let Some(built) = cache.index.get() else {
        return Err(helper_error("BuildError", "the index was not built"));
    };
    Ok(json!({ "chosen": built.name, "attempts": index.attempts() }))
}

async fn run_lookup(input: &LookupInput) -> HelperResult {
    let index = Arc::new(RecordingIndex {
        hits: input.hits.iter().map(to_hit).collect(),
        searches: Mutex::new(Vec::new()),
        stores: Mutex::new(Vec::new()),
    });
    let cache = helper_cache(
        Arc::new(ConstantEmbedder {
            vector: input.vector.clone(),
        }),
        index.clone(),
    )?;
    let mut metadata = input.metadata.as_ref().and_then(Json::as_object).cloned();
    let kwargs = helper_kwargs(
        input.messages.as_deref(),
        input.responses_input.as_ref(),
        input.metadata.as_ref(),
        None,
    );
    let value = cache
        .async_get_cache(&input.key, &kwargs, metadata.as_mut())
        .await
        .map_err(|error| helper_error("ValueError", error))?;
    let value = value
        .map(to_value)
        .transpose()
        .map_err(|error| helper_error("SerializationError", error))?;
    let similarity = similarity_of(metadata.as_ref())?;
    let search = index.searches().into_iter().next();
    Ok(json!({
        "value": value,
        "similarity": similarity,
        "search": search.map(|(_, query)| json!({
            "cache_key": query.cache_key,
            "vector": query.vector.iter().copied().map(f64::from).collect::<Vec<f64>>(),
            "distance_threshold": query.distance_threshold,
        })),
    }))
}

async fn run_store(input: &StoreInput) -> HelperResult {
    let index = Arc::new(RecordingIndex {
        hits: Vec::new(),
        searches: Mutex::new(Vec::new()),
        stores: Mutex::new(Vec::new()),
    });
    let cache = helper_cache(
        Arc::new(ConstantEmbedder {
            vector: input.vector.clone(),
        }),
        index.clone(),
    )?;
    let kwargs = helper_kwargs(
        input.messages.as_deref(),
        input.responses_input.as_ref(),
        input.metadata.as_ref(),
        input.ttl,
    );
    cache
        .async_set_cache(&input.key, &input.value, &kwargs)
        .await;
    let store = index.stores().into_iter().next().map(|(_, fields, ttl)| {
        let vector: Vec<f64> = fields
            .prompt_vector
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f64::from(f32::from_le_bytes(*bytes)))
            .collect();
        json!({
            "prompt": fields.prompt,
            "response": fields.response,
            "vector": vector,
            "filters": { CACHE_KEY_FIELD_NAME: fields.litellm_cache_key },
            "ttl": ttl,
        })
    });
    Ok(json!({ "store": store }))
}

fn similarity_of(metadata: Option<&Object>) -> Result<Option<Value>, Box<HelperError>> {
    let Some(value) = metadata.and_then(|map| map.get(SIMILARITY_FIELD)) else {
        return Ok(None);
    };
    Ok(Some(to_value(value).map_err(|error| {
        helper_error("SerializationError", error)
    })?))
}

fn helper_kwargs(
    messages: Option<&[Message]>,
    responses_input: Option<&Json>,
    metadata: Option<&Json>,
    ttl: Option<i64>,
) -> SemanticCacheKwargs {
    SemanticCacheKwargs {
        messages: messages.map(<[Message]>::to_vec),
        input: responses_input.cloned(),
        metadata: metadata.and_then(Json::as_object).cloned(),
        ttl,
    }
}

fn helper_cache(
    embedder: Arc<ConstantEmbedder>,
    index: Arc<dyn SemanticRedisIndex>,
) -> Result<RedisSemanticCache, Box<HelperError>> {
    let options = RedisSemanticOptions {
        host: Some("localhost".to_owned()),
        port: Some("6379".to_owned()),
        password: Some("pw".to_owned()),
        redis_url: None,
        similarity_threshold: Some(0.25),
        embedding_model: None,
        index_name: None,
        embedding_max_input_tokens: None,
        embedding_timeout: 5.0,
    };
    let env = RedisEnv {
        redis_host: None,
        redis_port: None,
        redis_password: None,
    };
    RedisSemanticCache::new(options, env, embedder, Arc::new(HelperConnector { index }))
        .map_err(|error| helper_error("ValueError", error))
}

fn user_message(content: &str) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn to_hit(input: &HitInput) -> Hit {
    Hit {
        entry_id: None,
        prompt: input.prompt.clone(),
        response: input.response.clone(),
        inserted_at: None,
        updated_at: None,
        metadata: None,
        litellm_cache_key: input.litellm_cache_key.clone(),
        vector_distance: input.vector_distance,
    }
}

struct ConstantEmbedder {
    vector: Vec<f32>,
}

#[async_trait]
impl SemanticEmbedder for ConstantEmbedder {
    async fn embed(
        &self,
        _prompt: &str,
        _metadata: Option<&Object>,
    ) -> Result<Vec<f32>, SemanticEmbedError> {
        Ok(self.vector.clone())
    }
}

struct UnusedEmbedder;

#[async_trait]
impl SemanticEmbedder for UnusedEmbedder {
    async fn embed(
        &self,
        _prompt: &str,
        _metadata: Option<&Object>,
    ) -> Result<Vec<f32>, SemanticEmbedError> {
        Err(SemanticEmbedError::Failed {
            message: "the constructor helper embeds nothing".to_owned(),
        })
    }
}

struct HelperConnector {
    index: Arc<dyn SemanticRedisIndex>,
}

#[async_trait]
impl SemanticIndexConnector for HelperConnector {
    async fn connect(
        &self,
        _redis_url: &str,
    ) -> Result<Arc<dyn SemanticRedisIndex>, RedisSemanticError> {
        Ok(Arc::clone(&self.index))
    }
}

struct UnusedConnector;

#[async_trait]
impl SemanticIndexConnector for UnusedConnector {
    async fn connect(
        &self,
        _redis_url: &str,
    ) -> Result<Arc<dyn SemanticRedisIndex>, RedisSemanticError> {
        Err(RedisSemanticError::MissingIndex {
            index: "the constructor helper connects nowhere".to_owned(),
        })
    }
}

struct RecordingIndex {
    hits: Vec<Hit>,
    searches: Mutex<Vec<(String, SemanticSearchQuery)>>,
    stores: Mutex<Vec<(String, CacheEntryFields, Option<i64>)>>,
}

impl RecordingIndex {
    fn searches(&self) -> Vec<(String, SemanticSearchQuery)> {
        self.searches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn stores(&self) -> Vec<(String, CacheEntryFields, Option<i64>)> {
        self.stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl SemanticRedisIndex for RecordingIndex {
    async fn ft_info(&self, _index: &str) -> Result<Option<Json>, RedisSemanticError> {
        Ok(None)
    }

    async fn ft_create(&self, _schema: &IndexSchema) -> Result<(), RedisSemanticError> {
        Ok(())
    }

    async fn ft_dropindex(&self, _index: &str) -> Result<(), RedisSemanticError> {
        Ok(())
    }

    async fn ft_search(
        &self,
        index: &str,
        query: &SemanticSearchQuery,
    ) -> Result<Vec<Hit>, RedisSemanticError> {
        self.searches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((index.to_owned(), query.clone()));
        Ok(self.hits.clone())
    }

    async fn store_hash(
        &self,
        key: &str,
        fields: &CacheEntryFields,
        ttl: Option<i64>,
    ) -> Result<(), RedisSemanticError> {
        self.stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), fields.clone(), ttl));
        Ok(())
    }
}

enum FallbackOp {
    Info(String),
    Drop(String),
}

struct ScriptedIndex {
    dimension: usize,
    mismatch_attempts: Vec<usize>,
    info_calls: Mutex<usize>,
    ops: Mutex<Vec<FallbackOp>>,
}

impl ScriptedIndex {
    fn attempts(&self) -> Vec<Value> {
        self.ops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|operation| match operation {
                FallbackOp::Info(name) => json!({ "name": name, "overwrite": false }),
                FallbackOp::Drop(name) => json!({ "name": name, "overwrite": true }),
            })
            .collect()
    }
}

#[async_trait]
impl SemanticRedisIndex for ScriptedIndex {
    async fn ft_info(&self, index: &str) -> Result<Option<Json>, RedisSemanticError> {
        let call = {
            let mut calls = self
                .info_calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let call = *calls;
            *calls += 1;
            call
        };
        self.ops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FallbackOp::Info(index.to_owned()));
        if self.mismatch_attempts.contains(&call) {
            return Ok(Some(info_reply("another-index", self.dimension)));
        }
        Ok(Some(info_reply(index, self.dimension)))
    }

    async fn ft_create(&self, _schema: &IndexSchema) -> Result<(), RedisSemanticError> {
        Ok(())
    }

    async fn ft_dropindex(&self, index: &str) -> Result<(), RedisSemanticError> {
        self.ops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(FallbackOp::Drop(index.to_owned()));
        Ok(())
    }

    async fn ft_search(
        &self,
        _index: &str,
        _query: &SemanticSearchQuery,
    ) -> Result<Vec<Hit>, RedisSemanticError> {
        Ok(Vec::new())
    }

    async fn store_hash(
        &self,
        _key: &str,
        _fields: &CacheEntryFields,
        _ttl: Option<i64>,
    ) -> Result<(), RedisSemanticError> {
        Ok(())
    }
}

fn info_reply(name: &str, dimension: usize) -> Json {
    let base_fields = [
        ("prompt", "TEXT"),
        ("response", "TEXT"),
        ("inserted_at", "NUMERIC"),
        ("updated_at", "NUMERIC"),
    ];
    let mut attributes = Vec::new();
    for (field, kind) in base_fields {
        attributes.push(field_attribute(field, kind));
    }
    attributes.push(vector_attribute(dimension));
    attributes.push(field_attribute(CACHE_KEY_FIELD_NAME, "TAG"));
    Json::List(vec![
        Json::Text("index_name".to_owned()),
        Json::Text(name.to_owned()),
        Json::Text("index_definition".to_owned()),
        Json::List(vec![
            Json::Text("key_type".to_owned()),
            Json::Text("HASH".to_owned()),
            Json::Text("prefixes".to_owned()),
            Json::List(vec![Json::Text(name.to_owned())]),
        ]),
        Json::Text("attributes".to_owned()),
        Json::List(attributes),
    ])
}

fn field_attribute(field: &str, kind: &str) -> Json {
    Json::List(vec![
        Json::Text("identifier".to_owned()),
        Json::Text(field.to_owned()),
        Json::Text("type".to_owned()),
        Json::Text(kind.to_owned()),
    ])
}

fn vector_attribute(dimension: usize) -> Json {
    Json::List(vec![
        Json::Text("identifier".to_owned()),
        Json::Text("prompt_vector".to_owned()),
        Json::Text("type".to_owned()),
        Json::Text("VECTOR".to_owned()),
        Json::Text("algorithm".to_owned()),
        Json::Text("FLAT".to_owned()),
        Json::Text("data_type".to_owned()),
        Json::Text("FLOAT32".to_owned()),
        Json::Text("dim".to_owned()),
        Json::Text(dimension.to_string()),
        Json::Text("distance_metric".to_owned()),
        Json::Text("COSINE".to_owned()),
    ])
}

fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: std::fmt::Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
