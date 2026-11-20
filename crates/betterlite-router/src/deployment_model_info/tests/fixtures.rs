use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::catalog_generalizations::CatalogGeneralizations;
use betterlite_models::fetch_policy::response::FetchResponse;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::MetadataEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use betterlite_models::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use betterlite_models::model_registration::{JsonObject, ModelCostEntries, ModelRegistry};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::provider_metadata_factory::adapters::DynamicModelInfo;
use betterlite_models::provider_model_sets::ProviderModelSets;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::compatible_models::{ModelList, ModelListCache, TokenLimits};
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::catalog_updates::catalog::{CatalogHandle, DeploymentCatalog};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_model_info::cache::DiscoveryCache;
use crate::deployment_model_info::discovery::CatalogSnapshot;
use crate::deployment_model_info::metadata::ModelMetadataSource;
use crate::deployment_model_info::router_info::SuppliedDeployment;
use crate::deployment_model_info::types::DiscoveredDeploymentModelInfo;
use crate::model_catalog_lookup::alias::AliasMap;
use crate::model_catalog_lookup::cache::CacheClear;
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::routing_groups::index::RoutingGroupIndex;

pub(super) const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

pub(super) fn spec(name: &str, params: &str, info: &str) -> DeploymentSpec {
    DeploymentSpec::new(
        name.to_owned(),
        object(params),
        object(info),
        JsonObject::new(),
        || "generated-id".to_owned(),
    )
    .unwrap()
}

pub(super) fn catalog(specs: Vec<DeploymentSpec>) -> Arc<DeploymentCatalog> {
    let mut catalog = DeploymentCatalog::default();
    for spec in specs {
        catalog.index_patterns(&spec).unwrap();
        catalog.push(spec);
    }
    Arc::new(catalog)
}

pub(super) fn snapshot(catalog: &Arc<DeploymentCatalog>) -> CatalogSnapshot<'_> {
    CatalogSnapshot { catalog }
}

pub(super) fn limits(
    max_tokens: Option<u64>,
    input: Option<u64>,
    output: Option<u64>,
) -> TokenLimits {
    TokenLimits {
        max_tokens,
        max_input_tokens: input,
        max_output_tokens: output,
    }
}

pub(super) fn record(
    catalog: &Arc<DeploymentCatalog>,
    index: usize,
    limits: TokenLimits,
) -> DiscoveredDeploymentModelInfo {
    DiscoveredDeploymentModelInfo {
        catalog: Arc::clone(catalog),
        index,
        limits,
    }
}

pub(super) fn dispatcher(seconds: f64) -> Arc<Mutex<f64>> {
    Arc::new(Mutex::new(seconds))
}

pub(super) fn clock(ticks: &Arc<Mutex<f64>>) -> Arc<dyn Fn() -> f64 + Send + Sync> {
    let handle = Arc::clone(ticks);
    Arc::new(move || *handle.lock().unwrap())
}

pub(super) fn registry(cost_map: &str) -> ModelRegistry {
    ModelRegistry {
        model_cost: object(cost_map),
        runtime_registered: ModelCostEntries::new(),
        provider_sets: ProviderModelSets::default(),
        keys: ModelCostKeys::default(),
        live_replay: None,
    }
}

pub(super) const fn no_inferred(_model: &str) -> Option<String> {
    None
}

#[derive(Default)]
pub(super) struct Lookups {
    pub(super) alias: AliasMap,
    pub(super) groups: RoutingGroupIndex,
}

impl Lookups {
    pub(super) fn lookup<'catalog>(
        &'catalog self,
        catalog: &'catalog DeploymentCatalog,
    ) -> CatalogLookup<'catalog> {
        CatalogLookup {
            catalog,
            model_group_alias: &self.alias,
            routing_groups: &self.groups,
            inferred_provider: &no_inferred,
        }
    }
}

pub(super) fn credentials(entries: &[(&str, &str)]) -> CredentialSnapshot {
    let mut built = Vec::new();
    for (name, values) in entries {
        built.push(CredentialEntry {
            credential_name: (*name).to_owned(),
            credential_values: object(values),
        });
    }
    CredentialSnapshot::new(built)
}

// The resolution inputs both fixture structs hold, so one context builder serves them.
pub(super) struct Inputs {
    pub(super) catalog: ModelCatalog,
    pub(super) known: BTreeSet<String>,
    pub(super) custom: BTreeSet<String>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) registry: JSONProviderRegistry,
}

impl Inputs {
    fn new(catalog: ModelCatalog, environment: BTreeMap<String, String>) -> Self {
        Self {
            catalog,
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment,
            registry: JSONProviderRegistry::load(),
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: self.registry.clone(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

pub(super) struct Environment {
    pub(super) map: ModelCostMap,
    pub(super) inputs: Inputs,
    pub(super) generalizations: CatalogGeneralizations,
}

impl Environment {
    pub(super) fn new(cost_map: &str, environment: &[(&str, &str)]) -> Self {
        let mut env = BTreeMap::new();
        for (key, value) in environment {
            env.insert((*key).to_owned(), (*value).to_owned());
        }
        let catalog = ModelCatalog::parse(cost_map.as_bytes(), UnknownFields::Warn).unwrap();
        let (generalizations, warnings) = CatalogGeneralizations::install(Vec::new());
        assert!(warnings.is_empty());
        Self {
            map: ModelCostMap::parse(cost_map.as_bytes()).unwrap(),
            inputs: Inputs::new(catalog, env),
            generalizations,
        }
    }

    pub(super) fn harness(&self) -> Harness<'_> {
        Harness::new(&self.map, self.inputs.context(), &self.generalizations)
    }
}

pub(super) struct Harness<'environment> {
    pub(super) map: &'environment ModelCostMap,
    pub(super) resolution: ResolutionContext<'environment>,
    pub(super) generalizations: &'environment CatalogGeneralizations,
}

impl<'environment> Harness<'environment> {
    #[must_use]
    const fn new(
        map: &'environment ModelCostMap,
        resolution: ResolutionContext<'environment>,
        generalizations: &'environment CatalogGeneralizations,
    ) -> Self {
        Self {
            map,
            resolution,
            generalizations,
        }
    }

    pub(super) fn metadata(&self) -> MetadataEnvironment<'_> {
        MetadataEnvironment {
            model_cost: &self.map.entries,
            vertex: &self.map.vertex,
            generalizations: self.generalizations,
            resolution: &self.resolution,
            normalizers: NORMALIZERS,
        }
    }
}

pub(super) fn plain_model(model: &str) -> String {
    model.to_owned()
}

pub(super) fn plain_split(model: &str) -> (Option<&str>, &str) {
    (None, model)
}

pub(super) const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: plain_model,
    bedrock_routing_prefix: plain_model,
    mantle_region_split: plain_split,
};

#[derive(Debug, thiserror::Error)]
#[error("the model is unmapped")]
pub(super) struct Unmapped;

// The scripted model info lookup: answers by name, records the names, and can fail instead.
pub(super) struct FakeSource {
    pub(super) answers: BTreeMap<String, JsonObject>,
    pub(super) failing: bool,
    pub(super) seen: Mutex<Vec<String>>,
}

impl FakeSource {
    pub(super) fn answering(answers: &[(&str, &str)]) -> Self {
        let mut map = BTreeMap::new();
        for (model, info) in answers {
            map.insert((*model).to_owned(), object(info));
        }
        Self {
            answers: map,
            failing: false,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing() -> Self {
        Self {
            answers: BTreeMap::new(),
            failing: true,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl ModelMetadataSource for FakeSource {
    type Error = Unmapped;

    fn get_model_info(
        &self,
        model: &str,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Unmapped>> + Send {
        self.seen.lock().unwrap().push(model.to_owned());
        let answer = if self.failing {
            Err(Unmapped)
        } else {
            Ok(self.answers.get(model).cloned())
        };
        ready(answer)
    }
}

// A dependency that only records the message of every event, with its level, for the log assertions.
#[derive(Clone, Default)]
pub(super) struct Events(Arc<Mutex<Vec<(String, String)>>>);

impl<S: Subscriber> Layer<S> for Events {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        // Only this crate's lines count: the HTTP client logs its own connect events at debug.
        if !event.metadata().target().starts_with("betterlite_router::") {
            return;
        }
        let mut field = MessageField::default();
        event.record(&mut field);
        if let Some(message) = field.0 {
            let level = event.metadata().level().to_string();
            self.0.lock().unwrap().push((level, message));
        }
    }
}

#[derive(Default)]
struct MessageField(Option<String>);

impl Visit for MessageField {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl Events {
    pub(super) fn messages(&self) -> Vec<(String, String)> {
        self.0.lock().unwrap().clone()
    }

    pub(super) fn at(&self, level: &str) -> Vec<String> {
        self.messages()
            .into_iter()
            .filter(|(seen, _)| seen == level)
            .map(|(_, message)| message)
            .collect()
    }
}

#[derive(Default)]
pub(super) struct Recorder {
    held: Mutex<BTreeMap<String, ModelList>>,
    stored: Mutex<Vec<(String, Duration)>>,
}

impl ModelListCache for Recorder {
    fn get(&self, key: &str) -> Option<ModelList> {
        self.held.lock().unwrap().get(key).cloned()
    }

    fn set(&self, key: &str, list: ModelList, ttl: Duration) {
        self.held.lock().unwrap().insert(key.to_owned(), list);
        self.stored.lock().unwrap().push((key.to_owned(), ttl));
    }
}

impl Recorder {
    pub(super) fn stored(&self) -> Vec<(String, Duration)> {
        self.stored.lock().unwrap().clone()
    }
}

#[derive(Default)]
pub(super) struct Counting(AtomicUsize);

impl CacheClear for Counting {
    fn clear(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Counting {
    pub(super) fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

pub(super) struct FakeHf {
    pub(super) body: Option<&'static str>,
}

impl ModelInfoClient for FakeHf {
    type Error = io::Error;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, io::Error>> + Send {
        let answer = self.body.map_or_else(
            || Err(io::Error::other("no config")),
            |body| {
                Ok(FetchResponse::new(
                    200,
                    Vec::new(),
                    body.as_bytes().to_vec(),
                ))
            },
        );
        ready(answer)
    }
}

pub(super) fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

pub(super) fn authorization(requests: &[String], path: &str) -> Option<String> {
    let sent = requests.iter().find(|sent| sent.contains(path))?;
    let lower = sent.to_lowercase();
    let (_prefix, rest) = lower.split_once("authorization: ")?;
    Some(rest.lines().next().unwrap_or_default().trim().to_owned())
}

pub(super) async fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        request.extend_from_slice(&chunk[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8(request).unwrap();
        }
    }
}

async fn respond(stream: &mut TcpStream, status: u16, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
}

// Publish the replacement before the response, so the worker's snapshot is already stale when it writes.
pub(super) async fn answer_replacing(
    mut stream: TcpStream,
    handle: &CatalogHandle,
    replacement: Arc<DeploymentCatalog>,
    body: &[u8],
) -> String {
    let request = read_request(&mut stream).await;
    handle.store(replacement);
    respond(&mut stream, 200, body).await;
    request
}

pub(super) async fn accept_connections(listener: &TcpListener, count: usize) -> Vec<TcpStream> {
    let mut streams = Vec::new();
    for _ in 0..count {
        let (stream, _) = listener.accept().await.unwrap();
        streams.push(stream);
    }
    streams
}

pub(super) async fn answer_connections(streams: Vec<TcpStream>, body: &[u8]) -> Vec<String> {
    let mut requests = Vec::new();
    for mut stream in streams {
        let request = read_request(&mut stream).await;
        respond(&mut stream, 200, body).await;
        requests.push(request);
    }
    requests
}

// The resolution inputs the deployment calls read, with the name sets the tests leave empty.
pub(super) struct Resolver {
    inputs: Inputs,
}

impl Resolver {
    pub(super) fn new(environment: &[(&str, &str)]) -> Self {
        Self::with_known(environment, &[])
    }

    pub(super) fn with_known(environment: &[(&str, &str)], known: &[&str]) -> Self {
        let mut built = BTreeMap::new();
        for (key, value) in environment {
            built.insert((*key).to_owned(), (*value).to_owned());
        }
        let catalog = ModelCatalog::parse(b"{}", UnknownFields::Reject).unwrap();
        let mut inputs = Inputs::new(catalog, built);
        inputs.known = known.iter().map(|name| (*name).to_owned()).collect();
        Self { inputs }
    }

    pub(super) fn context(&self) -> ResolutionContext<'_> {
        self.inputs.context()
    }
}

// A dynamic metadata source with nothing to say, so the resolution reaches the cost map and the Hugging Face fetch.
pub(super) struct NoDynamic;

impl DynamicMetadataSource for NoDynamic {
    type Error = Infallible;

    fn get_model_info(
        &self,
        _adapter: DynamicModelInfo,
        _model: &str,
        _api_base: Option<&str>,
        _api_key: Option<&SecretString>,
    ) -> impl Future<Output = Result<Option<Box<RawValue>>, Infallible>> + Send {
        let answer = Ok(None);
        ready(answer)
    }
}

pub(super) fn capture(events: Events) -> impl Subscriber + Send + Sync {
    tracing_subscriber::registry().with(events)
}

pub(super) fn discovery_with(
    catalog: &Arc<DeploymentCatalog>,
    index: usize,
    limits: TokenLimits,
) -> DiscoveryCache {
    let spec = &catalog.models[index];
    let model_id = spec.id().unwrap();
    let cache = DiscoveryCache::for_deployments(catalog.models.len());
    cache.store(model_id, record(catalog, index, limits));
    cache
}

impl<'row> SuppliedDeployment<'row> {
    #[must_use]
    pub(super) const fn from_spec(spec: &'row DeploymentSpec) -> Self {
        Self {
            model_info: &spec.model_info,
            litellm_params: Some(&spec.litellm_params),
        }
    }
}
