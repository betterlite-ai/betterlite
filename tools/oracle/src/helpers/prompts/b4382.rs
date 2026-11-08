mod fakes;
mod input;

use std::sync::Arc;

use secrecy::ExposeSecret as _;
use serde_json::{Value, json, to_value};

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::message_content::{Content, Message};
use betterlite_prompts::redis_semantic_cache::options::{RedisEnv, RedisSemanticOptions};
use betterlite_prompts::redis_semantic_cache::ports::{Hit, SemanticRedisIndex};
use betterlite_prompts::redis_semantic_cache::{
    CACHE_KEY_FIELD_NAME, RedisSemanticCache, SIMILARITY_FIELD, SemanticCacheKwargs,
};

use self::fakes::{ConstantEmbedder, HelperConnector, HelperIndex};
use self::input::{HitInput, Input, Operation};
use crate::helpers::{Helper, HelperError, HelperResult};

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
    match input.operation {
        Operation::Constructor => constructor(input),
        Operation::KeyScope => key_scope(input),
        Operation::IndexFallback => index_fallback(input).await,
        Operation::Lookup => run_lookup(input).await,
        Operation::Store => run_store(input).await,
    }
}

fn constructor(input: &Input) -> HelperResult {
    let options = RedisSemanticOptions {
        host: input.host.clone(),
        port: input.port.clone(),
        password: input.password.clone(),
        redis_url: input.redis_url.clone(),
        similarity_threshold: input.threshold,
        embedding_model: input.embedding_model.clone(),
        index_name: input.index_name.clone(),
        embedding_max_input_tokens: input.embedding_max_input_tokens,
        // B4310's resolve_embedding_timeout default, which the Python adapter applies too.
        embedding_timeout: input.embedding_timeout.unwrap_or(5.0_f64),
    };
    let env = RedisEnv {
        redis_host: input.redis_host.clone(),
        redis_port: input.redis_port.clone(),
        redis_password: input.redis_password.clone(),
    };
    let embedder = ConstantEmbedder { vector: Vec::new() };
    let backend: Arc<dyn SemanticRedisIndex> = Arc::new(HelperIndex::with_hits(Vec::new()));
    let cache = RedisSemanticCache::new(
        options,
        env,
        Arc::new(embedder),
        Arc::new(HelperConnector { index: backend }),
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

fn key_scope(input: &Input) -> HelperResult {
    let key = required(input.key.as_ref(), "key")?;
    let candidate = Hit {
        prompt: None,
        response: None,
        litellm_cache_key: input.litellm_cache_key.clone(),
        vector_distance: None,
    };
    Ok(json!({ "matches": RedisSemanticCache::cache_hit_matches_key(&candidate, &key) }))
}

async fn index_fallback(input: &Input) -> HelperResult {
    let index_name = required(input.index_name.as_ref(), "index_name")?;
    let dimension = required(input.dimension.as_ref(), "dimension")?;
    let index = Arc::new(HelperIndex::scripted(
        dimension,
        input.mismatch_attempts.clone(),
    ));
    let embedder = ConstantEmbedder {
        vector: vec![0.5; dimension],
    };
    let concrete = Arc::clone(&index);
    let backend: Arc<dyn SemanticRedisIndex> = concrete;
    let cache = helper_cache(Arc::new(embedder), backend, Some(index_name))?;
    let messages = [user_message("fallback")];
    let kwargs = helper_kwargs(Some(&messages), None, None, None);
    cache.async_set_cache("scope-1", &Json::Null, &kwargs).await;
    let Some(built) = cache.index.get() else {
        return Err(helper_error("BuildError", "the index was not built"));
    };
    Ok(json!({ "chosen": built.name, "attempts": index.attempts() }))
}

struct ScriptedCall {
    key: String,
    cache: RedisSemanticCache,
    index: Arc<HelperIndex>,
    kwargs: SemanticCacheKwargs,
}

fn scripted_call(
    input: &Input,
    hits: Vec<Hit>,
    ttl: Option<i64>,
) -> Result<ScriptedCall, Box<HelperError>> {
    let key = required(input.key.as_ref(), "key")?;
    let vector = required(input.vector.as_ref(), "vector")?;
    let index = Arc::new(HelperIndex::with_hits(hits));
    let embedder = ConstantEmbedder { vector };
    let concrete = Arc::clone(&index);
    let backend: Arc<dyn SemanticRedisIndex> = concrete;
    let cache = helper_cache(Arc::new(embedder), backend, None)?;
    let kwargs = helper_kwargs(
        input.messages.as_deref(),
        input.responses.as_ref(),
        input.metadata.as_ref(),
        ttl,
    );
    Ok(ScriptedCall {
        key,
        cache,
        index,
        kwargs,
    })
}

async fn run_lookup(input: &Input) -> HelperResult {
    let call = scripted_call(input, input.hits.iter().map(to_hit).collect(), None)?;
    let mut metadata = input.metadata.as_ref().and_then(Json::as_object).cloned();
    let value = call
        .cache
        .async_get_cache(&call.key, &call.kwargs, metadata.as_mut())
        .await
        .map_err(|error| helper_error("ValueError", error))?;
    let decoded = value
        .map(to_value)
        .transpose()
        .map_err(|error| helper_error("SerializationError", error))?;
    let similarity = similarity_of(metadata.as_ref())?;
    let search = call.index.searches().into_iter().next();
    Ok(json!({
        "value": decoded,
        "similarity": similarity,
        "search": search.map(|(_, query)| json!({
            "cache_key": query.cache_key,
            "vector": query.vector.iter().copied().map(f64::from).collect::<Vec<f64>>(),
            "distance_threshold": query.distance_threshold,
        })),
    }))
}

async fn run_store(input: &Input) -> HelperResult {
    let call = scripted_call(input, Vec::new(), input.ttl)?;
    let value = required(input.value.as_ref(), "value")?;
    call.cache
        .async_set_cache(&call.key, &value, &call.kwargs)
        .await;
    Ok(json!({ "store": store_record(&call.index) }))
}

fn store_record(index: &HelperIndex) -> Option<Value> {
    let (_, fields, ttl) = index.stores().into_iter().next()?;
    let vector: Vec<f64> = fields
        .prompt_vector
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f64::from(f32::from_le_bytes(*bytes)))
        .collect();
    Some(json!({
        "prompt": fields.prompt,
        "response": fields.response,
        "vector": vector,
        "filters": { CACHE_KEY_FIELD_NAME: fields.litellm_cache_key },
        "ttl": ttl,
    }))
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
    index_name: Option<String>,
) -> Result<RedisSemanticCache, Box<HelperError>> {
    let options = RedisSemanticOptions {
        host: Some("localhost".to_owned()),
        port: Some("6379".to_owned()),
        password: Some("pw".to_owned()),
        redis_url: None,
        similarity_threshold: Some(0.25_f64),
        embedding_model: None,
        index_name,
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
        prompt: input.prompt.clone(),
        response: input.response.clone(),
        litellm_cache_key: input.litellm_cache_key.clone(),
        vector_distance: input.vector_distance,
    }
}

fn required<T>(value: Option<&T>, name: &str) -> Result<T, Box<HelperError>>
where
    T: Clone,
{
    value
        .cloned()
        .ok_or_else(|| helper_error("InvalidInput", format!("{name} is required")))
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
