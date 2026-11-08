use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_redis::redis_live_client::LiveClientError;
use serde_json::Number;

use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, Message};
use crate::redis_semantic_cache::error::RedisSemanticError;
use crate::redis_semantic_cache::options::{RedisEnv, RedisSemanticOptions};
use crate::redis_semantic_cache::ports::{
    CacheEntryFields, Hit, IndexSchema, SemanticClock, SemanticEmbedError, SemanticEmbedder,
    SemanticIndexConnector, SemanticRedisIndex, SemanticSearchQuery,
};
use crate::redis_semantic_cache::{CACHE_KEY_FIELD_NAME, RedisSemanticCache, SemanticCacheKwargs};

pub(crate) const PROBE: [f32; 3] = [0.5, 0.5, 0.5];
pub(crate) const VECTOR: [f32; 3] = [0.25, 0.5, 0.75];
pub(crate) const TIMESTAMP: f64 = 1_700_000_000.5;

pub(crate) fn options() -> RedisSemanticOptions {
    RedisSemanticOptions {
        host: Some("localhost".to_owned()),
        port: Some("6379".to_owned()),
        password: Some("pw".to_owned()),
        redis_url: None,
        similarity_threshold: Some(0.25),
        embedding_model: None,
        index_name: None,
        embedding_max_input_tokens: None,
        embedding_timeout: 5.0,
    }
}

pub(crate) fn empty_env() -> RedisEnv {
    RedisEnv {
        redis_host: None,
        redis_port: None,
        redis_password: None,
    }
}

pub(crate) fn user_message(content: &str) -> Message {
    Message {
        role: "user".to_owned(),
        content: Some(Content::Text(content.to_owned())),
        name: None,
        provider_specific_fields: Object::new(),
    }
}

pub(crate) struct FakeEmbedder {
    pub(crate) vector: Vec<f32>,
    probe: Vec<f32>,
    failure: Option<String>,
}

impl FakeEmbedder {
    pub(crate) fn new() -> Self {
        Self {
            vector: VECTOR.to_vec(),
            probe: PROBE.to_vec(),
            failure: None,
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            vector: VECTOR.to_vec(),
            probe: PROBE.to_vec(),
            failure: Some("the embedding provider failed".to_owned()),
        }
    }
}

#[async_trait]
impl SemanticEmbedder for FakeEmbedder {
    async fn embed(
        &self,
        prompt: &str,
        _metadata: Option<&Object>,
    ) -> Result<Vec<f32>, SemanticEmbedError> {
        if let Some(message) = &self.failure {
            return Err(SemanticEmbedError::Failed {
                message: message.clone(),
            });
        }
        if prompt == "dimension test" {
            return Ok(self.probe.clone());
        }
        Ok(self.vector.clone())
    }
}

pub(crate) struct FixedClock(pub(crate) f64);

impl SemanticClock for FixedClock {
    fn now_seconds(&self) -> Result<f64, RedisSemanticError> {
        Ok(self.0)
    }
}

#[derive(Default)]
pub(crate) struct FakeIndex {
    infos: Mutex<BTreeMap<String, Json>>,
    pub(crate) info_failure: Mutex<Option<RedisSemanticError>>,
    create_failure: Mutex<Option<RedisSemanticError>>,
    pub(crate) store_failure: Mutex<Option<RedisSemanticError>>,
    creates: Mutex<Vec<IndexSchema>>,
    drops: Mutex<Vec<String>>,
    searches: Mutex<Vec<(String, SemanticSearchQuery)>>,
    stores: Mutex<Vec<(String, CacheEntryFields, Option<i64>)>>,
    hits: Mutex<Vec<Hit>>,
}

impl FakeIndex {
    pub(crate) fn with_info(self, name: &str, reply: Json) -> Self {
        self.infos
            .lock()
            .expect("the info table")
            .insert(name.to_owned(), reply);
        self
    }

    pub(crate) fn creates(&self) -> Vec<IndexSchema> {
        self.creates.lock().expect("the create log").clone()
    }

    pub(crate) fn drops(&self) -> Vec<String> {
        self.drops.lock().expect("the drop log").clone()
    }

    pub(crate) fn searches(&self) -> Vec<(String, SemanticSearchQuery)> {
        self.searches.lock().expect("the search log").clone()
    }

    pub(crate) fn stores(&self) -> Vec<(String, CacheEntryFields, Option<i64>)> {
        self.stores.lock().expect("the store log").clone()
    }

    pub(crate) fn set_hits(&self, hits: Vec<Hit>) {
        *self.hits.lock().expect("the hit list") = hits;
    }
}

#[async_trait]
impl SemanticRedisIndex for FakeIndex {
    async fn ft_info(&self, index: &str) -> Result<Option<Json>, RedisSemanticError> {
        let failure = self.info_failure.lock().expect("the info failure").take();
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(self
            .infos
            .lock()
            .expect("the info table")
            .get(index)
            .cloned())
    }

    async fn ft_create(&self, schema: &IndexSchema) -> Result<(), RedisSemanticError> {
        let failure = self
            .create_failure
            .lock()
            .expect("the create failure")
            .take();
        if let Some(error) = failure {
            return Err(error);
        }
        self.creates
            .lock()
            .expect("the create log")
            .push(schema.clone());
        Ok(())
    }

    async fn ft_dropindex(&self, index: &str) -> Result<(), RedisSemanticError> {
        self.drops
            .lock()
            .expect("the drop log")
            .push(index.to_owned());
        Ok(())
    }

    async fn ft_search(
        &self,
        index: &str,
        query: &SemanticSearchQuery,
    ) -> Result<Vec<Hit>, RedisSemanticError> {
        self.searches
            .lock()
            .expect("the search log")
            .push((index.to_owned(), query.clone()));
        Ok(self.hits.lock().expect("the hit list").clone())
    }

    async fn store_hash(
        &self,
        key: &str,
        fields: &CacheEntryFields,
        ttl: Option<i64>,
    ) -> Result<(), RedisSemanticError> {
        let failure = self.store_failure.lock().expect("the store failure").take();
        if let Some(error) = failure {
            return Err(error);
        }
        self.stores
            .lock()
            .expect("the store log")
            .push((key.to_owned(), fields.clone(), ttl));
        Ok(())
    }
}

pub(crate) struct FakeConnector {
    index: Arc<FakeIndex>,
    connects: Mutex<Vec<String>>,
    pub(crate) failure: Mutex<Option<RedisSemanticError>>,
}

impl FakeConnector {
    pub(crate) fn new(index: Arc<FakeIndex>) -> Self {
        Self {
            index,
            connects: Mutex::new(Vec::new()),
            failure: Mutex::new(None),
        }
    }

    pub(crate) fn connects(&self) -> usize {
        self.connects.lock().expect("the connect log").len()
    }
}

#[async_trait]
impl SemanticIndexConnector for FakeConnector {
    async fn connect(
        &self,
        redis_url: &str,
    ) -> Result<Arc<dyn SemanticRedisIndex>, RedisSemanticError> {
        self.connects
            .lock()
            .expect("the connect log")
            .push(redis_url.to_owned());
        let failure = self.failure.lock().expect("the connect failure").take();
        if let Some(error) = failure {
            return Err(error);
        }
        let backend = Arc::clone(&self.index);
        Ok(backend)
    }
}

pub(crate) fn cache_with(
    embedder: FakeEmbedder,
    index: Arc<FakeIndex>,
) -> (RedisSemanticCache, Arc<FakeConnector>) {
    let connector = Arc::new(FakeConnector::new(index));
    let embedder_handle: Arc<dyn SemanticEmbedder> = Arc::new(embedder);
    let index_handle = Arc::clone(&connector);
    let mut cache = RedisSemanticCache::new(options(), empty_env(), embedder_handle, index_handle)
        .expect("the cache settings");
    cache.clock = Arc::new(FixedClock(TIMESTAMP));
    (cache, connector)
}

pub(crate) fn stored_cache() -> (RedisSemanticCache, Arc<FakeIndex>) {
    let index = Arc::new(FakeIndex::default());
    let (cache, _connector) = cache_with(FakeEmbedder::new(), Arc::clone(&index));
    (cache, index)
}

pub(crate) fn kwargs_with_message(content: &str) -> SemanticCacheKwargs {
    SemanticCacheKwargs {
        messages: Some(vec![user_message(content)]),
        input: None,
        metadata: None,
        ttl: None,
    }
}

pub(crate) fn hit(distance: f64, key: Option<&str>, response: &str) -> Hit {
    Hit {
        prompt: Some("the prompt".to_owned()),
        response: Some(response.to_owned()),
        litellm_cache_key: key.map(str::to_owned),
        vector_distance: Some(distance),
    }
}

pub(crate) fn alternative_error() -> RedisSemanticError {
    RedisSemanticError::Backend(LiveClientError::InvalidSetting { field: "info" })
}

fn attribute(name: &str, kind: &str, extra: &[(&str, String)]) -> Json {
    let mut fields = vec![
        Json::Text("identifier".to_owned()),
        Json::Text(name.to_owned()),
        Json::Text("attribute".to_owned()),
        Json::Text(name.to_owned()),
        Json::Text("type".to_owned()),
        Json::Text(kind.to_owned()),
    ];
    for (key, value) in extra {
        fields.push(Json::Text((*key).to_owned()));
        fields.push(Json::Text(value.clone()));
    }
    Json::List(fields)
}

pub(crate) fn info_reply(name: &str, dimension: usize) -> Json {
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
        Json::List(vec![
            attribute("prompt", "TEXT", &[]),
            attribute("response", "TEXT", &[]),
            attribute("inserted_at", "NUMERIC", &[]),
            attribute("updated_at", "NUMERIC", &[]),
            attribute(
                "prompt_vector",
                "VECTOR",
                &[
                    ("algorithm", "FLAT".to_owned()),
                    ("data_type", "FLOAT32".to_owned()),
                    ("dim", dimension.to_string()),
                    ("distance_metric", "COSINE".to_owned()),
                ],
            ),
            attribute(CACHE_KEY_FIELD_NAME, "TAG", &[]),
        ]),
        Json::Text("num_docs".to_owned()),
        Json::Number(Number::from(3_i64)),
    ])
}

pub(crate) fn command_args(command: &redis::Cmd) -> Vec<Vec<u8>> {
    command
        .args_iter()
        .map(|argument| {
            if let redis::Arg::Simple(bytes) = argument {
                bytes.to_vec()
            } else {
                panic!("a literal command carries no cursor argument");
            }
        })
        .collect()
}

pub(crate) fn text_args(command: &redis::Cmd) -> Vec<String> {
    command_args(command)
        .into_iter()
        .map(|bytes| String::from_utf8(bytes).expect("a text argument"))
        .collect()
}
