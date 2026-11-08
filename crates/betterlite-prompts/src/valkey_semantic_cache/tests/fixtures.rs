use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_redis::redis_live_client::LiveClientError;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, Message};
use crate::redis_semantic_cache::SemanticCacheKwargs;
use crate::redis_semantic_cache::ports::{SemanticEmbedError, SemanticEmbedder};
use crate::valkey_semantic_cache::ValkeySemanticCache;
use crate::valkey_semantic_cache::error::ValkeySemanticError;
use crate::valkey_semantic_cache::options::{ValkeyEnv, ValkeySemanticOptions};
use crate::valkey_semantic_cache::ports::{
    DocIdSource, DocMapping, Hit, IndexSchema, KnnQuery, ValkeyConnector, ValkeyIndex,
};
use crate::valkey_semantic_cache::schema::pack_vector;

pub(crate) const VECTOR: [f32; 3] = [0.25, 0.5, 0.75];
pub(crate) const PROMPT: &str = "what is the weather";
pub(crate) const INDEX_NAME: &str = "litellm_semantic_cache_index";

fn take_failure<T>(slot: &Mutex<Option<T>>) -> Option<T> {
    slot.lock().expect("a scripted failure").take()
}

pub(crate) fn options() -> ValkeySemanticOptions {
    ValkeySemanticOptions {
        host: Some("localhost".to_owned()),
        port: Some("6379".to_owned()),
        password: Some(SecretString::from("pw")),
        redis_url: None,
        similarity_threshold: Some(0.25),
        embedding_model: None,
        index_name: None,
        embedding_max_input_tokens: None,
        embedding_timeout: 5.0,
        ssl: false,
        startup_nodes: Vec::new(),
        index: None,
    }
}

pub(crate) fn empty_env() -> ValkeyEnv {
    ValkeyEnv {
        valkey_host: None,
        valkey_port: None,
        valkey_password: None,
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

pub(crate) fn build(
    settings: ValkeySemanticOptions,
    env: ValkeyEnv,
) -> Result<ValkeySemanticCache, ValkeySemanticError> {
    ValkeySemanticCache::new(
        settings,
        env,
        Arc::new(FakeEmbedder::new()),
        Arc::new(FakeConnector::new(Arc::new(FakeIndex::default()))),
    )
}

pub(crate) struct FakeEmbedder {
    vector: Mutex<Vec<f32>>,
    calls: Mutex<Vec<(String, Option<Object>)>>,
    failure: Mutex<Option<String>>,
}

impl FakeEmbedder {
    pub(crate) fn new() -> Self {
        Self::with_vector(VECTOR.to_vec())
    }

    pub(crate) fn with_vector(vector: Vec<f32>) -> Self {
        Self {
            vector: Mutex::new(vector),
            calls: Mutex::default(),
            failure: Mutex::default(),
        }
    }

    pub(crate) fn calls(&self) -> Vec<(String, Option<Object>)> {
        self.calls.lock().expect("the embed log").clone()
    }

    pub(crate) fn set_vector(&self, vector: Vec<f32>) {
        *self.vector.lock().expect("the vector") = vector;
    }

    pub(crate) fn fail(&self) {
        *self.failure.lock().expect("the failure") =
            Some("the embedding provider failed".to_owned());
    }
}

#[async_trait]
impl SemanticEmbedder for FakeEmbedder {
    async fn embed(
        &self,
        prompt: &str,
        metadata: Option<&Object>,
    ) -> Result<Vec<f32>, SemanticEmbedError> {
        self.calls
            .lock()
            .expect("the embed log")
            .push((prompt.to_owned(), metadata.cloned()));
        if let Some(message) = take_failure(&self.failure) {
            return Err(SemanticEmbedError::Failed { message });
        }
        Ok(self.vector.lock().expect("the vector").clone())
    }
}

#[derive(Default)]
pub(crate) struct FakeDocIds {
    next: Mutex<u64>,
}

impl DocIdSource for FakeDocIds {
    fn next_id(&self) -> String {
        let mut next = self.next.lock().expect("the id counter");
        let id = format!("00000000-0000-4000-8000-{next:012}");
        *next = next.wrapping_add(1);
        id
    }
}

#[derive(Default)]
pub(crate) struct FakeIndex {
    infos: Mutex<BTreeMap<String, Json>>,
    create_errors: Mutex<VecDeque<ValkeySemanticError>>,
    info_error: Mutex<Option<ValkeySemanticError>>,
    search_error: Mutex<Option<ValkeySemanticError>>,
    store_error: Mutex<Option<ValkeySemanticError>>,
    creates: Mutex<Vec<(String, IndexSchema)>>,
    searches: Mutex<Vec<(String, KnnQuery, Vec<u8>)>>,
    stores: Mutex<Vec<(String, DocMapping, Option<i64>)>>,
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

    pub(crate) fn fail_create_next(&self, error: ValkeySemanticError) {
        self.create_errors
            .lock()
            .expect("the create errors")
            .push_back(error);
    }

    pub(crate) fn fail_info(&self, error: ValkeySemanticError) {
        *self.info_error.lock().expect("the info error") = Some(error);
    }

    pub(crate) fn fail_search(&self, error: ValkeySemanticError) {
        *self.search_error.lock().expect("the search error") = Some(error);
    }

    pub(crate) fn fail_store(&self, error: ValkeySemanticError) {
        *self.store_error.lock().expect("the store error") = Some(error);
    }

    pub(crate) fn creates(&self) -> Vec<(String, IndexSchema)> {
        self.creates.lock().expect("the create log").clone()
    }

    pub(crate) fn searches(&self) -> Vec<(String, KnnQuery, Vec<u8>)> {
        self.searches.lock().expect("the search log").clone()
    }

    pub(crate) fn stores(&self) -> Vec<(String, DocMapping, Option<i64>)> {
        self.stores.lock().expect("the store log").clone()
    }

    pub(crate) fn set_hits(&self, hits: Vec<Hit>) {
        *self.hits.lock().expect("the hit list") = hits;
    }
}

#[async_trait]
impl ValkeyIndex for FakeIndex {
    async fn ft_create(
        &self,
        index: &str,
        schema: &IndexSchema,
    ) -> Result<(), ValkeySemanticError> {
        self.creates
            .lock()
            .expect("the create log")
            .push((index.to_owned(), schema.clone()));
        let scripted = self
            .create_errors
            .lock()
            .expect("the create errors")
            .pop_front();
        scripted.map_or(Ok(()), Err)
    }

    async fn ft_info(&self, index: &str) -> Result<Json, ValkeySemanticError> {
        let reply = self
            .infos
            .lock()
            .expect("the info table")
            .get(index)
            .cloned();
        if let Some(error) = take_failure(&self.info_error) {
            return Err(error);
        }
        reply.ok_or_else(alternative_error)
    }

    async fn ft_search(
        &self,
        index: &str,
        query: &KnnQuery,
        vector: &[u8],
    ) -> Result<Vec<Hit>, ValkeySemanticError> {
        let failure = take_failure(&self.search_error);
        self.searches.lock().expect("the search log").push((
            index.to_owned(),
            query.clone(),
            vector.to_vec(),
        ));
        failure.map_or_else(|| Ok(self.hits.lock().expect("the hit list").clone()), Err)
    }

    async fn store_hash(
        &self,
        key: &str,
        fields: &DocMapping,
        ttl: Option<i64>,
    ) -> Result<(), ValkeySemanticError> {
        if let Some(error) = take_failure(&self.store_error) {
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
    failure: Mutex<Option<ValkeySemanticError>>,
}

impl FakeConnector {
    pub(crate) fn new(index: Arc<FakeIndex>) -> Self {
        Self {
            index,
            connects: Mutex::default(),
            failure: Mutex::default(),
        }
    }

    pub(crate) fn fail(&self, error: ValkeySemanticError) {
        *self.failure.lock().expect("the connect failure") = Some(error);
    }

    pub(crate) fn connects(&self) -> Vec<String> {
        self.connects.lock().expect("the connect log").clone()
    }
}

#[async_trait]
impl ValkeyConnector for FakeConnector {
    async fn connect(
        &self,
        url: &SecretString,
    ) -> Result<Arc<dyn ValkeyIndex>, ValkeySemanticError> {
        self.connects
            .lock()
            .expect("the connect log")
            .push(url.expose_secret().to_owned());
        if let Some(error) = take_failure(&self.failure) {
            return Err(error);
        }
        let backend = Arc::clone(&self.index);
        Ok(backend)
    }
}

pub(crate) fn cache_with(
    embedder: Arc<FakeEmbedder>,
    index: &Arc<FakeIndex>,
) -> (ValkeySemanticCache, Arc<FakeConnector>) {
    let connector = Arc::new(FakeConnector::new(Arc::clone(index)));
    let embedder_handle: Arc<dyn SemanticEmbedder> = embedder;
    let connector_concrete = Arc::clone(&connector);
    let connector_handle: Arc<dyn ValkeyConnector> = connector_concrete;
    let mut cache =
        ValkeySemanticCache::new(options(), empty_env(), embedder_handle, connector_handle)
            .expect("the cache settings");
    cache.doc_ids = Arc::new(FakeDocIds::default());
    (cache, connector)
}

pub(crate) fn cache_with_injected_index(
    index: &Arc<FakeIndex>,
) -> (ValkeySemanticCache, Arc<FakeConnector>) {
    let mut settings = options();
    let injected = Arc::clone(index);
    let backend: Arc<dyn ValkeyIndex> = injected;
    settings.index = Some(backend);
    settings.host = None;
    settings.port = None;
    settings.password = None;
    let connector = Arc::new(FakeConnector::new(Arc::clone(index)));
    let embedder_handle: Arc<dyn SemanticEmbedder> = Arc::new(FakeEmbedder::new());
    let connector_concrete = Arc::clone(&connector);
    let connector_handle: Arc<dyn ValkeyConnector> = connector_concrete;
    let mut cache =
        ValkeySemanticCache::new(settings, empty_env(), embedder_handle, connector_handle)
            .expect("the injected index");
    cache.doc_ids = Arc::new(FakeDocIds::default());
    (cache, connector)
}

pub(crate) fn kwargs_with_message(content: &str) -> SemanticCacheKwargs {
    SemanticCacheKwargs {
        messages: Some(vec![user_message(content)]),
        input: None,
        metadata: None,
        ttl: None,
    }
}

pub(crate) fn kwargs_without_prompt() -> SemanticCacheKwargs {
    SemanticCacheKwargs {
        messages: None,
        input: None,
        metadata: None,
        ttl: None,
    }
}

pub(crate) fn hit(distance: f64, response: &str) -> Hit {
    Hit {
        distance,
        response: response.to_owned(),
    }
}

pub(crate) fn alternative_error() -> ValkeySemanticError {
    ValkeySemanticError::Backend(LiveClientError::InvalidSetting { field: "create" })
}

pub(crate) fn server_error(message: &str) -> ValkeySemanticError {
    ValkeySemanticError::Backend(LiveClientError::Redis(redis::RedisError::from((
        redis::ErrorKind::Client,
        "ERR",
        message.to_owned(),
    ))))
}

pub(crate) fn exists_error() -> ValkeySemanticError {
    server_error("Index already exists")
}

pub(crate) fn vector_attribute(dimension: usize) -> Json {
    let mut fields = vec![
        Json::Text("identifier".to_owned()),
        Json::Text("embedding".to_owned()),
        Json::Text("attribute".to_owned()),
        Json::Text("embedding".to_owned()),
        Json::Text("type".to_owned()),
        Json::Text("VECTOR".to_owned()),
    ];
    let marker: u64 = u64::try_from(dimension).expect("a dimension");
    fields.push(Json::List(vec![
        Json::Text("dimensions".to_owned()),
        Json::Number(Number::from(marker)),
        Json::Text("distance_metric".to_owned()),
        Json::Text("COSINE".to_owned()),
    ]));
    Json::List(fields)
}

pub(crate) fn info_reply(dimension: usize) -> Json {
    let mut info = Object::new();
    info.insert("index_name".to_owned(), Json::Text(INDEX_NAME.to_owned()));
    info.insert(
        "attributes".to_owned(),
        Json::List(vec![vector_attribute(dimension)]),
    );
    info.insert("num_docs".to_owned(), Json::Number(Number::from(2_i64)));
    Json::Object(info)
}

pub(crate) fn command_args(command: &redis::Cmd) -> Vec<Vec<u8>> {
    command
        .args_iter()
        .map(|argument| match argument {
            redis::Arg::Simple(bytes) => bytes.to_vec(),
            redis::Arg::Cursor => panic!("a literal command carries no cursor argument"),
            _ => panic!("an argument kind this fixture does not know"),
        })
        .collect()
}

pub(crate) fn text_args(command: &redis::Cmd) -> Vec<String> {
    command_args(command)
        .into_iter()
        .map(|bytes| String::from_utf8(bytes).expect("a text argument"))
        .collect()
}

pub(crate) fn packed_vector() -> Vec<u8> {
    pack_vector(&VECTOR)
}
