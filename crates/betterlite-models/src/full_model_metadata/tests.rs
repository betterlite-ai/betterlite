#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::convert::Infallible;
use std::future::{Future, ready};
use std::io;
use std::sync::{Mutex, PoisonError};

use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use serde_json::value::RawValue;

use super::cache::{MetadataCaches, ModelInfoCache};
use super::error::FullMetadataError;
use super::parameters::SupportedParameterSource;
use super::record::ModelInfoHandle;
use super::{
    FullMetadataSources, HelperQuery, HelperSources, get_model_info, get_model_info_helper_cached,
};
use crate::catalog::{ModelCatalog, UnknownFields};
use crate::catalog_generalizations::CatalogGeneralizations;
use crate::decisions::Json;
use crate::entry::ModelFields;
use crate::fetch_policy::response::FetchResponse;
use crate::fireworks_catalog_metadata::Catalog as FireworksCatalog;
use crate::json_provider_registry::JSONProviderRegistry;
use crate::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use crate::model_catalog_keys::ModelCostKeys;
use crate::model_metadata_resolution::dynamic::DynamicMetadataSource;
use crate::model_metadata_resolution::hf_config::HfConfigClient;
use crate::model_metadata_resolution::names::{ModelCostMap, NameNormalizers};
use crate::model_metadata_resolution::{MetadataEnvironment, MetadataRequest};
use crate::params::capabilities::OpenAiCapability;
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_b::globals::GlobalCredentials;
use crate::provider_metadata_factory::adapters::DynamicModelInfo;
use crate::provider_metadata_factory::selection::MetadataQuery;
use crate::resolution::ResolutionContext;

mod cases;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: plain_model,
    bedrock_routing_prefix: plain_model,
    mantle_region_split: plain_split,
};

fn plain_model(model: &str) -> String {
    model.to_owned()
}

fn plain_split(model: &str) -> (Option<&str>, &str) {
    (None, model)
}

const OPENAI_MODELS: &str = r#"{
    "gpt-4o": {"litellm_provider": "openai", "mode": "chat", "input_cost_per_token": 1, "output_cost_per_token": 2},
    "gpt-4": {"litellm_provider": "openai", "mode": "chat", "input_cost_per_token": 1, "output_cost_per_token": 2},
    "gpt-3.5-turbo": {"litellm_provider": "openai", "mode": "chat", "input_cost_per_token": 1, "output_cost_per_token": 2},
    "llama3.2": {"litellm_provider": "ollama", "mode": "chat", "input_cost_per_token": 1, "output_cost_per_token": 2}
}"#;

fn raw(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.to_owned()).expect("raw JSON")
}

struct Fixture {
    map: ModelCostMap,
    catalog: ModelCatalog,
    fireworks: FireworksCatalog,
    environment: BTreeMap<String, String>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
}

impl Fixture {
    fn new(source: &str) -> Self {
        let map = ModelCostMap::parse(source.as_bytes()).expect("model cost map");
        let catalog =
            ModelCatalog::parse(source.as_bytes(), UnknownFields::Warn).expect("model catalog");
        let fireworks = map
            .entries
            .iter()
            .filter_map(|(key, value)| {
                let fields: ModelFields = serde_json::from_str(value.get()).ok()?;
                Some((key.clone(), fields))
            })
            .collect();
        Self {
            map,
            catalog,
            fireworks,
            environment: BTreeMap::new(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
        }
    }

    fn harness(&self) -> Harness<'_> {
        let (generalizations, warnings) = CatalogGeneralizations::install(Vec::new());
        assert!(warnings.is_empty());
        Harness {
            fixture: self,
            resolution: ResolutionContext {
                catalog: &self.catalog,
                known_model_names: &self.known_model_names,
                custom_providers: &self.custom_providers,
                environment: &self.environment,
                registry: JSONProviderRegistry::load(),
                policies: ProviderPolicies::default(),
                globals: GlobalCredentials::default(),
            },
            generalizations,
        }
    }

    fn replace(&mut self, key: &str, entry: &str) {
        self.map.entries.insert(key.to_owned(), raw(entry));
    }
}

struct Harness<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    generalizations: CatalogGeneralizations,
}

impl Harness<'_> {
    fn environment(&self) -> MetadataEnvironment<'_> {
        MetadataEnvironment {
            model_cost: &self.fixture.map.entries,
            vertex: &self.fixture.map.vertex,
            generalizations: &self.generalizations,
            resolution: &self.resolution,
            normalizers: NORMALIZERS,
        }
    }
}

struct State {
    keys: ModelCostKeys,
    caches: MetadataCaches,
    dynamic: ScriptedDynamic,
    hf: HfConfigClient<NoConfig>,
}

impl State {
    fn new(maxsize: i64, dynamic: ScriptedDynamic) -> Self {
        Self {
            keys: ModelCostKeys::default(),
            caches: MetadataCaches {
                full: ModelInfoCache::new(maxsize),
                helper: ModelInfoCache::new(maxsize),
            },
            dynamic,
            hf: HfConfigClient {
                client: NoConfig,
                base_url: String::new(),
            },
        }
    }
}

async fn full_call<Parameters>(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &Parameters,
    request: &MetadataRequest<'_>,
) -> Result<ModelInfoHandle, FullMetadataError<Parameters::Error>>
where
    Parameters: SupportedParameterSource,
{
    let environment = harness.environment();
    let sources = FullMetadataSources {
        helper: HelperSources {
            environment: &environment,
            dynamic: &state.dynamic,
            hf: &state.hf,
        },
        provider_query: MetadataQuery {
            catalog: &harness.fixture.fireworks,
            generation: state.keys.generation,
        },
        parameters,
    };
    get_model_info(request, &sources, &mut state.keys, &state.caches).await
}

async fn helper_call(
    harness: &Harness<'_>,
    state: &mut State,
    query: &HelperQuery<'_>,
) -> Result<ModelInfoHandle, FullMetadataError<Infallible>> {
    let environment = harness.environment();
    let sources = HelperSources {
        environment: &environment,
        dynamic: &state.dynamic,
        hf: &state.hf,
    };
    get_model_info_helper_cached(query, &sources, &mut state.keys, &state.caches.helper).await
}

fn request<'request>(
    model: &'request str,
    provider: Option<&'request str>,
    api_base: Option<&'request str>,
    api_key: Option<&'request SecretString>,
) -> MetadataRequest<'request> {
    MetadataRequest {
        model,
        custom_llm_provider: provider,
        api_base,
        api_key,
    }
}

fn field(handle: &ModelInfoHandle, name: &str) -> Option<Json> {
    handle
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fields
        .get(name)
        .cloned()
}

fn set_field(handle: &ModelInfoHandle, name: &str, value: Json) {
    let mut info = handle.lock().unwrap_or_else(PoisonError::into_inner);
    drop(info.fields.insert(name.to_owned(), value));
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[derive(Debug)]
enum ParameterAnswer {
    Answer(Option<Vec<String>>),
    Failure,
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted parameter query failed")]
struct ParameterFailure;

#[derive(Debug, Default)]
struct ScriptedParameters {
    answers: Mutex<VecDeque<ParameterAnswer>>,
    calls: Mutex<Vec<(String, Option<String>)>>,
}

impl ScriptedParameters {
    fn new(answers: Vec<ParameterAnswer>) -> Self {
        Self {
            answers: Mutex::new(VecDeque::from(answers)),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<(String, Option<String>)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl SupportedParameterSource for ScriptedParameters {
    type Error = ParameterFailure;

    fn supported_openai_params(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, ParameterFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), custom_llm_provider.map(str::to_owned)));
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match answer {
            Some(ParameterAnswer::Answer(value)) => Ok(value),
            Some(ParameterAnswer::Failure) => Err(ParameterFailure),
            None => Ok(None),
        }
    }
}

#[derive(Debug)]
enum Reply {
    Value(Box<RawValue>),
    None,
}

#[derive(Clone, Debug)]
struct DynamicCall {
    adapter: DynamicModelInfo,
    model: String,
    api_base: Option<String>,
    authenticated: bool,
}

#[derive(Debug, Default)]
struct ScriptedDynamic {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<DynamicCall>>,
}

impl ScriptedDynamic {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<DynamicCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl DynamicMetadataSource for ScriptedDynamic {
    type Error = io::Error;

    fn get_model_info(
        &self,
        adapter: DynamicModelInfo,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> impl Future<Output = Result<Option<Box<RawValue>>, io::Error>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(DynamicCall {
                adapter,
                model: model.to_owned(),
                api_base: api_base.map(str::to_owned),
                authenticated: api_key.is_some(),
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Reply::None);
        ready(match reply {
            Reply::Value(value) => Ok(Some(value)),
            Reply::None => Ok(None),
        })
    }
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("no Hugging Face config transport is configured")]
struct NoConfig;

impl ModelInfoClient for NoConfig {
    type Error = Self;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self>> + Send {
        ready(Err(Self))
    }
}

fn openai_capabilities() -> BTreeSet<OpenAiCapability> {
    BTreeSet::from([OpenAiCapability::CatalogModel])
}

#[derive(Clone, Copy, Debug)]
enum Case {
    FullQueryOrder,
    ProviderOverlay,
    CredentialCacheBypass,
    HelperAndFullCaches,
    InvalidationGeneration,
    CachedRecordMutation,
}

#[rstest]
#[case::full_query_order(Case::FullQueryOrder)]
#[case::provider_overlay(Case::ProviderOverlay)]
#[case::credential_cache_bypass(Case::CredentialCacheBypass)]
#[case::helper_and_full_caches(Case::HelperAndFullCaches)]
#[case::invalidation_generation(Case::InvalidationGeneration)]
#[case::cached_record_mutation(Case::CachedRecordMutation)]
#[tokio::test]
async fn full_model_metadata_cases(#[case] case: Case) {
    match case {
        Case::FullQueryOrder => cases::test_full_query_order_case().await,
        Case::ProviderOverlay => cases::test_provider_overlay_case().await,
        Case::CredentialCacheBypass => cases::test_credential_cache_bypass_case().await,
        Case::HelperAndFullCaches => cases::test_helper_and_full_caches_case().await,
        Case::InvalidationGeneration => cases::test_invalidation_generation_case().await,
        Case::CachedRecordMutation => cases::test_cached_record_mutation_case().await,
    }
}
