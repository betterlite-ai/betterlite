#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use serde_json::{Value, json};

use super::dynamic::DynamicMetadataSource;
use super::error::ModelMetadataError;
use super::hf_config::{HfConfigClient, decode_hf_model_config};
use super::names::{
    ModelCostMap, ModelCostMapError, NameNormalizers, PotentialNamesInput, VertexModelSets,
    potential_model_names,
};
use super::{MetadataEnvironment, MetadataRequest, get_model_info_helper};
use crate::catalog::{ModelCatalog, UnknownFields};
use crate::catalog_generalizations::CatalogGeneralizations;
use crate::fetch_policy::response::FetchResponse;
use crate::json_provider_registry::JSONProviderRegistry;
use crate::lemonade_model_metadata::{ModelInfoClient, ModelInfoRequest};
use crate::model_catalog_keys::{ModelCostKeys, PotentialModelNames};
use crate::provider_defaults_a::policies::ProviderPolicies;
use crate::provider_defaults_b::globals::GlobalCredentials;
use crate::provider_metadata_factory::adapters::DynamicModelInfo;
use crate::resolution::ResolutionContext;

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: test_bedrock_base_model,
    bedrock_routing_prefix: test_bedrock_routing_prefix,
    mantle_region_split: test_mantle_region_split,
};

fn test_bedrock_base_model(model: &str) -> String {
    let stripped = model.strip_prefix("converse/").unwrap_or(model);
    let regional = stripped.strip_prefix("us.").unwrap_or(stripped);
    regional.to_owned()
}

fn test_bedrock_routing_prefix(model: &str) -> String {
    match model.strip_prefix("converse/") {
        Some(rest) => rest.to_owned(),
        None => model.to_owned(),
    }
}

fn test_mantle_region_split(model: &str) -> (Option<&str>, &str) {
    match model.split_once('/') {
        Some((head, rest)) if head == "us-east-1" => (Some(head), rest),
        _ => (None, model),
    }
}

struct Fixture {
    map: ModelCostMap,
    catalog: ModelCatalog,
    environment: BTreeMap<String, String>,
    known_model_names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    rules: Vec<Box<RawValue>>,
}

impl Fixture {
    fn new(source: &str) -> Self {
        let mut fixture = Self::raw(source);
        fixture.catalog =
            ModelCatalog::parse(source.as_bytes(), UnknownFields::Warn).expect("catalog");
        fixture
    }

    // The cost map keeps entries the typed catalog rejects, as LiteLLM's dict would.
    fn raw(source: &str) -> Self {
        let map = ModelCostMap::parse(source.as_bytes()).expect("model cost map");
        let catalog = ModelCatalog::parse(b"{}", UnknownFields::Reject).expect("empty catalog");
        Fixture {
            map,
            catalog,
            environment: BTreeMap::new(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            rules: Vec::new(),
        }
    }

    fn with_rules(raw: &str, catalog: &str, rules: &str) -> Self {
        let mut fixture = Self::new(catalog);
        fixture.map = ModelCostMap::parse(raw.as_bytes()).expect("model cost map");
        fixture.rules = serde_json::from_str(rules).expect("generalization rules");
        fixture
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known_model_names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn generalizations(&self) -> CatalogGeneralizations {
        let (installed, warnings) = CatalogGeneralizations::install(self.rules.clone());
        assert!(warnings.is_empty(), "{warnings:?}");
        installed
    }
}

struct Environment<'fixture> {
    fixture: &'fixture Fixture,
    resolution: ResolutionContext<'fixture>,
    generalizations: CatalogGeneralizations,
}

impl<'fixture> Environment<'fixture> {
    fn new(fixture: &'fixture Fixture) -> Self {
        Environment {
            fixture,
            resolution: fixture.resolution(),
            generalizations: fixture.generalizations(),
        }
    }

    fn metadata(&self) -> MetadataEnvironment<'_> {
        MetadataEnvironment {
            model_cost: &self.fixture.map.entries,
            vertex: &self.fixture.map.vertex,
            generalizations: &self.generalizations,
            resolution: &self.resolution,
            normalizers: NORMALIZERS,
        }
    }
}

fn names(fixture: &Fixture, model: &str, provider: Option<&str>) -> PotentialModelNames {
    let resolution = fixture.resolution();
    potential_model_names(&PotentialNamesInput {
        model,
        custom_llm_provider: provider,
        resolution: &resolution,
        normalizers: NORMALIZERS,
    })
}

fn request<'request>(
    model: &'request str,
    provider: Option<&'request str>,
) -> MetadataRequest<'request> {
    MetadataRequest {
        model,
        custom_llm_provider: provider,
        api_base: None,
        api_key: None,
    }
}

async fn resolve<Source: DynamicMetadataSource>(
    fixture: &Fixture,
    model: &str,
    provider: Option<&str>,
    dynamic: &Source,
) -> Result<Value, ModelMetadataError> {
    let environment = Environment::new(fixture);
    let hf = HfConfigClient::unreachable();
    let mut keys = ModelCostKeys::default();
    get_model_info_helper(
        &request(model, provider),
        &environment.metadata(),
        &mut keys,
        dynamic,
        &hf,
    )
    .await
    .map(|raw| serde_json::from_str(raw.get()).expect("projected JSON"))
}

#[derive(Default)]
struct NeverDynamic;

impl DynamicMetadataSource for NeverDynamic {
    type Error = Infallible;

    fn get_model_info(
        &self,
        _adapter: DynamicModelInfo,
        _model: &str,
        _api_base: Option<&str>,
        _api_key: Option<&SecretString>,
    ) -> impl Future<Output = Result<Option<Box<RawValue>>, Infallible>> + Send {
        std::future::ready(Ok(None))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DynamicCall {
    adapter: DynamicModelInfo,
    model: String,
    api_base: Option<String>,
    api_key: Option<String>,
}

#[derive(Clone, Debug)]
enum Reply {
    Value(Value),
    None,
    Error,
}

struct ScriptedDynamic {
    replies: Mutex<VecDeque<Reply>>,
    calls: Mutex<Vec<DynamicCall>>,
}

impl ScriptedDynamic {
    fn new(replies: Vec<Reply>) -> Self {
        ScriptedDynamic {
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
                api_key: api_key.map(|key| key.expose_secret().to_owned()),
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Reply::None);
        std::future::ready(match reply {
            Reply::Value(value) => Ok(Some(serde_json::value::to_raw_value(&value).expect("raw"))),
            Reply::None => Ok(None),
            Reply::Error => Err(io::Error::other("dynamic metadata failed")),
        })
    }
}

#[derive(Clone, Debug)]
enum HttpReply {
    Status { status: u16, body: String },
    Failure,
    Slow,
}

#[derive(Default)]
struct ScriptedTransport {
    replies: Mutex<VecDeque<HttpReply>>,
    seen: Mutex<Vec<ModelInfoRequest>>,
}

impl ScriptedTransport {
    fn new(replies: Vec<HttpReply>) -> Self {
        ScriptedTransport {
            replies: Mutex::new(VecDeque::from(replies)),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<ModelInfoRequest> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ModelInfoClient for &ScriptedTransport {
    type Error = io::Error;

    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, io::Error>> + Send {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(HttpReply::Failure);
        async move {
            match reply {
                HttpReply::Status { status, body } => {
                    Ok(FetchResponse::new(status, Vec::new(), body.into_bytes()))
                }
                HttpReply::Failure => Err(io::Error::other("config fetch failed")),
                HttpReply::Slow => {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Ok(FetchResponse::new(200, Vec::new(), b"{}".to_vec()))
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    PotentialNames,
    ProviderAliases,
    DynamicProviderMetadata,
    OrderedExactLookup,
    FillMissingExactEntry,
    GeneralizationOnlyUnknown,
    UnmappedErrorBoundary,
    HuggingfaceConfigMetadata,
}

#[rstest]
#[case::potential_names(Scenario::PotentialNames)]
#[case::provider_aliases(Scenario::ProviderAliases)]
#[case::dynamic_provider_metadata(Scenario::DynamicProviderMetadata)]
#[case::ordered_exact_lookup(Scenario::OrderedExactLookup)]
#[case::fill_missing_exact_entry(Scenario::FillMissingExactEntry)]
#[case::generalization_only_unknown(Scenario::GeneralizationOnlyUnknown)]
#[case::unmapped_error_boundary(Scenario::UnmappedErrorBoundary)]
#[case::huggingface_config_metadata(Scenario::HuggingfaceConfigMetadata)]
#[tokio::test]
async fn model_metadata_resolution_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::PotentialNames => potential_names_case().await,
        Scenario::ProviderAliases => provider_aliases_case().await,
        Scenario::DynamicProviderMetadata => dynamic_provider_metadata_case().await,
        Scenario::OrderedExactLookup => ordered_exact_lookup_case().await,
        Scenario::FillMissingExactEntry => fill_missing_exact_entry_case().await,
        Scenario::GeneralizationOnlyUnknown => generalization_only_unknown_case().await,
        Scenario::UnmappedErrorBoundary => unmapped_error_boundary_case().await,
        Scenario::HuggingfaceConfigMetadata => huggingface_config_metadata_case().await,
    }
}

async fn potential_names_case() {
    let fixture = Fixture::new(
        r#"{
            "gpt-4o": {"litellm_provider": "openai", "mode": "chat"},
            "vertex-model": {"litellm_provider": "vertex_ai-chat-models", "mode": "chat"}
        }"#,
    );
    let inferred = names(&fixture, "gpt-4o", None);
    assert_eq!(inferred.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(inferred.split_model, "gpt-4o");
    assert_eq!(inferred.combined_model_name, "gpt-4o");
    assert_eq!(inferred.stripped_model_name, "gpt-4o");
    assert_eq!(inferred.combined_stripped_model_name, "gpt-4o");
    assert_eq!(inferred.provider_prefixed_model_name, "gpt-4o");

    let unknown = names(&fixture, "not-anywhere-xyz", None);
    assert_eq!(unknown.custom_llm_provider, None);
    assert_eq!(unknown.split_model, "not-anywhere-xyz");
    assert_eq!(unknown.combined_model_name, "not-anywhere-xyz");
    assert_eq!(unknown.provider_prefixed_model_name, "not-anywhere-xyz");

    let prefixed = names(&fixture, "groq/llama3-8b", Some("groq"));
    assert_eq!(prefixed.split_model, "llama3-8b");
    assert_eq!(prefixed.combined_model_name, "groq/llama3-8b");
    assert_eq!(prefixed.stripped_model_name, "llama3-8b");
    assert_eq!(prefixed.combined_stripped_model_name, "groq/llama3-8b");
    assert_eq!(prefixed.provider_prefixed_model_name, "groq/groq/llama3-8b");

    let dated = names(&fixture, "llama3-8b-2024-01-01", Some("groq"));
    assert_eq!(dated.split_model, "llama3-8b-2024-01-01");
    assert_eq!(dated.combined_model_name, "groq/llama3-8b-2024-01-01");
    assert_eq!(dated.stripped_model_name, "llama3-8b");
    assert_eq!(dated.combined_stripped_model_name, "groq/llama3-8b");
    assert_eq!(
        dated.provider_prefixed_model_name,
        "groq/llama3-8b-2024-01-01"
    );

    let bedrock = names(&fixture, "converse/anthropic.claude-3", Some("bedrock"));
    assert_eq!(bedrock.split_model, "anthropic.claude-3");
    assert_eq!(
        bedrock.combined_model_name,
        "bedrock/converse/anthropic.claude-3"
    );
    assert_eq!(bedrock.stripped_model_name, "anthropic.claude-3");
    assert_eq!(
        bedrock.combined_stripped_model_name,
        "bedrock/anthropic.claude-3"
    );

    let mantle = names(&fixture, "us-east-1/some-model", Some("bedrock_mantle"));
    assert_eq!(mantle.split_model, "some-model");
    assert_eq!(
        mantle.combined_model_name,
        "bedrock_mantle/us-east-1/some-model"
    );
    assert_eq!(
        mantle.combined_stripped_model_name,
        "bedrock_mantle/some-model"
    );
    assert_eq!(
        mantle.provider_prefixed_model_name,
        "bedrock_mantle/us-east-1/some-model"
    );

    let fireworks = names(&fixture, "llama-v3p1-8b", Some("fireworks_ai"));
    assert_eq!(fireworks.combined_model_name, "fireworks_ai/llama-v3p1-8b");
    assert_eq!(
        fireworks.provider_prefixed_model_name,
        "fireworks_ai/accounts/fireworks/models/llama-v3p1-8b"
    );
}

async fn provider_aliases_case() {
    let fixture = Fixture::new(
        r#"{
            "azure/gpt-35-turbo": {"litellm_provider": "azure", "mode": "chat", "input_cost_per_token": 1},
            "azure/ada": {"litellm_provider": "azure", "mode": "embedding", "input_cost_per_token": 2},
            "vertex_ai/meta/llama3-8b-instruct-maas": {"litellm_provider": "vertex_ai-llama_models", "mode": "chat", "input_cost_per_token": 3},
            "vertex_ai/codestral@latest": {"litellm_provider": "vertex_ai-mistral_models", "mode": "chat", "input_cost_per_token": 4},
            "vertex_ai/jamba-1.5@latest": {"litellm_provider": "vertex_ai-ai21_models", "mode": "chat", "input_cost_per_token": 5},
            "github_copilot/gpt-4": {"litellm_provider": "github_copilot", "mode": "chat", "input_cost_per_token": 6}
        }"#,
    );
    let azure = resolve(&fixture, "gpt-35-turbo", None, &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(azure["key"], "azure/gpt-35-turbo");
    let embedding = resolve(&fixture, "ada", None, &NeverDynamic).await.unwrap();
    assert_eq!(embedding["key"], "azure/ada");
    let beta = resolve(
        &fixture,
        "llama3-8b-instruct-maas",
        Some("vertex_ai_beta"),
        &NeverDynamic,
    )
    .await
    .unwrap();
    assert_eq!(beta["key"], "vertex_ai/meta/llama3-8b-instruct-maas");
    let mistral = resolve(&fixture, "codestral", Some("vertex_ai"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(mistral["key"], "vertex_ai/codestral@latest");
    let ai21 = resolve(&fixture, "jamba-1.5", Some("vertex_ai"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(ai21["key"], "vertex_ai/jamba-1.5@latest");
    let declared = resolve(&fixture, "github_copilot/gpt-4", None, &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(declared["key"], "github_copilot/gpt-4");
}

async fn dynamic_provider_metadata_case() {
    let fixture = Fixture::new(
        r#"{
            "ollama/llama3.2": {"litellm_provider": "ollama", "mode": "chat", "input_cost_per_token": 9},
            "openai/gpt-4o": {"litellm_provider": "openai", "mode": "chat", "input_cost_per_token": 7}
        }"#,
    );
    let dynamic = ScriptedDynamic::new(vec![Reply::Value(
        json!({"key": "runtime", "max_tokens": 32}),
    )]);
    let value = resolve(&fixture, "llama3.2", Some("ollama"), &dynamic)
        .await
        .unwrap();
    assert_eq!(value["key"], "runtime");
    assert_eq!(
        dynamic.calls(),
        vec![DynamicCall {
            adapter: DynamicModelInfo::Ollama,
            model: "llama3.2".to_owned(),
            api_base: None,
            api_key: None,
        }]
    );

    let alias = ScriptedDynamic::new(vec![Reply::Value(json!({"key": "rewritten"}))]);
    let rewritten = resolve(&fixture, "ada", Some("ollama"), &alias)
        .await
        .unwrap();
    assert_eq!(rewritten["key"], "rewritten");
    assert_eq!(alias.calls()[0].model, "azure/ada");

    let falling_back = ScriptedDynamic::new(vec![Reply::Error]);
    let fallback = resolve(&fixture, "llama3.2", Some("ollama"), &falling_back)
        .await
        .unwrap();
    assert_eq!(fallback["key"], "ollama/llama3.2");

    let none = ScriptedDynamic::new(vec![Reply::None]);
    let static_entry = resolve(&fixture, "llama3.2", Some("ollama"), &none)
        .await
        .unwrap();
    assert_eq!(static_entry["key"], "ollama/llama3.2");

    let unused = ScriptedDynamic::new(vec![Reply::Value(json!({"key": "ignored"}))]);
    let selected = resolve(&fixture, "openai/gpt-4o", Some("openai"), &unused)
        .await
        .unwrap();
    assert_eq!(selected["key"], "openai/gpt-4o");
    assert!(unused.calls().is_empty());
}

async fn ordered_exact_lookup_case() {
    let fixture = Fixture::new(
        r#"{
            "groq/llama3-8b-2024-01-01": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 1},
            "groq/llama3-8b": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 2},
            "llama3-8b-2024-01-01": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 3},
            "llama3-8b": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 4},
            "groq/groq/llama3-8b-2024-01-01": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 5}
        }"#,
    );
    let combined = resolve(
        &fixture,
        "groq/llama3-8b-2024-01-01",
        Some("groq"),
        &NeverDynamic,
    )
    .await
    .unwrap();
    assert_eq!(combined["key"], "groq/llama3-8b-2024-01-01");
    assert_eq!(combined["input_cost_per_token"], 1);

    let mismatched = Fixture::new(
        r#"{
            "groq/llama3-8b": {"litellm_provider": "openai", "mode": "chat", "input_cost_per_token": 1},
            "llama3-8b": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 2}
        }"#,
    );
    let continued = resolve(&mismatched, "llama3-8b", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(continued["key"], "llama3-8b");
    assert_eq!(continued["input_cost_per_token"], 2);

    let casefolded = Fixture::new(
        r#"{"GROQ/Llama3-8B": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 6}}"#,
    );
    let folded = resolve(&casefolded, "groq/llama3-8b", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(folded["key"], "GROQ/Llama3-8B");
    assert_eq!(folded["input_cost_per_token"], 6);

    let rules = r#"[{"pattern": "^groq/", "model_info": {"input_cost_per_token": 99}}]"#;
    let ordered = Fixture::with_rules(
        r#"{"llama3-8b": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 9}}"#,
        r#"{"llama3-8b": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 9}}"#,
        rules,
    );
    let exact = resolve(&ordered, "llama3-8b", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(exact["key"], "llama3-8b");
    assert_eq!(exact["input_cost_per_token"], 9);

    let split_only = Fixture::new(
        r#"{"llama3-8b": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 10}}"#,
    );
    let split = resolve(&split_only, "groq/llama3-8b", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(split["key"], "llama3-8b");
}

async fn fill_missing_exact_entry_case() {
    let rules = r#"[{
        "pattern": "^groq/",
        "fill_missing_for_providers": ["groq"],
        "model_info": {"max_tokens": 100, "supports_vision": true, "input_cost_per_token": 99, "mode": "completion"}
    }]"#;
    let fixture = Fixture::with_rules(
        r#"{
            "groq/model": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 1},
            "groq/absent-mode": {"litellm_provider": "groq", "input_cost_per_token": 1},
            "groq/responses": {"litellm_provider": "groq", "mode": "responses", "input_cost_per_token": 1},
            "groq/completion": {"litellm_provider": "groq", "mode": "completion", "input_cost_per_token": 1},
            "groq/null-mode": {"litellm_provider": "groq", "mode": null, "input_cost_per_token": 1},
            "groq/explicit": {"litellm_provider": "groq", "mode": "chat", "max_tokens": null, "supports_vision": false, "rpm": 0, "input_cost_per_token": 1}
        }"#,
        r#"{
            "groq/model": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 1},
            "groq/absent-mode": {"litellm_provider": "groq", "input_cost_per_token": 1},
            "groq/responses": {"litellm_provider": "groq", "mode": "responses", "input_cost_per_token": 1},
            "groq/completion": {"litellm_provider": "groq", "mode": "completion", "input_cost_per_token": 1},
            "groq/null-mode": {"litellm_provider": "groq", "mode": null, "input_cost_per_token": 1},
            "groq/explicit": {"litellm_provider": "groq", "mode": "chat", "max_tokens": null, "supports_vision": false, "rpm": 0, "input_cost_per_token": 1}
        }"#,
        rules,
    );
    let chat = resolve(&fixture, "model", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(chat["key"], "groq/model");
    assert_eq!(chat["max_tokens"], 100);
    assert_eq!(chat["supports_vision"], true);
    assert_eq!(chat["input_cost_per_token"], 1);
    assert_eq!(chat["mode"], "chat");

    let absent = resolve(&fixture, "absent-mode", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(absent["max_tokens"], 100);
    let responses = resolve(&fixture, "responses", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(responses["max_tokens"], 100);
    let completion = resolve(&fixture, "completion", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(completion["max_tokens"], Value::Null);
    let null_mode = resolve(&fixture, "null-mode", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(null_mode["max_tokens"], Value::Null);

    let explicit = resolve(&fixture, "explicit", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(explicit["max_tokens"], Value::Null);
    assert_eq!(explicit["supports_vision"], false);
    assert_eq!(explicit["rpm"], 0);
    assert_eq!(
        fixture.map.entries["groq/model"].get(),
        r#"{"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 1}"#
    );
}

async fn generalization_only_unknown_case() {
    let rules = r#"[
        {"pattern": "^anthropic/claude-future$", "model_info": {"max_tokens": 4096, "litellm_provider": "anthropic"}},
        {"pattern": "^mystery-model$", "model_info": {"max_tokens": 7, "litellm_provider": "acme"}},
        {"pattern": "^empty-rule$", "model_info": {}},
        {"pattern": "^anthropic/blocked", "model_info": {"max_tokens": 11}}
    ]"#;
    let fixture = Fixture::with_rules(
        r#"{"anthropic/blocked-prefix": {"litellm_provider": "openai", "mode": "chat"}}"#,
        r#"{"anthropic/blocked-prefix": {"litellm_provider": "openai", "mode": "chat"}}"#,
        rules,
    );
    let projected = resolve(&fixture, "claude-future", Some("anthropic"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(projected["key"], "anthropic/claude-future");
    assert_eq!(projected["max_tokens"], 4096);
    assert_eq!(projected["litellm_provider"], "anthropic");

    let retained = resolve(&fixture, "mystery-model-2024-01-01", None, &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(retained["key"], "mystery-model");
    assert_eq!(retained["max_tokens"], 7);
    assert_eq!(retained["litellm_provider"], "acme");

    let empty = resolve(&fixture, "empty-rule", Some("acme"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(empty["key"], "empty-rule");
    assert_eq!(empty["litellm_provider"], "acme");
    assert_eq!(empty["max_tokens"], Value::Null);

    let blocked = resolve(&fixture, "blocked-prefix", Some("anthropic"), &NeverDynamic).await;
    assert!(matches!(blocked, Err(ModelMetadataError::NotMapped { .. })));
}

async fn unmapped_error_boundary_case() {
    let fixture = Fixture::new(r#"{"azure/gpt-35-turbo": {"litellm_provider": "azure"}}"#);
    let missing = resolve(&fixture, "nowhere-model", Some("groq"), &NeverDynamic).await;
    match missing {
        Err(error @ ModelMetadataError::NotMapped { .. }) => {
            assert_eq!(
                error.to_string(),
                "This model isn't mapped yet. model=nowhere-model, custom_llm_provider=groq. Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json."
            );
        }
        other => panic!("expected a not-mapped error, got {other:?}"),
    }

    // LiteLLM's dict keeps an entry the typed catalog rejects, and its prefix branch raises on a non-text provider
    let malformed =
        Fixture::raw(r#"{"vertex_ai/bad-model": {"litellm_provider": 5, "mode": "chat"}}"#);
    let failure = resolve(
        &malformed,
        "vertex_ai/bad-model",
        Some("vertex_ai"),
        &NeverDynamic,
    )
    .await;
    match failure {
        Err(error @ ModelMetadataError::Failed { .. }) => {
            assert_eq!(
                error.to_string(),
                "This model isn't mapped yet. model=vertex_ai/bad-model, custom_llm_provider=vertex_ai. Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json."
            );
        }
        other => panic!("expected a generic failure, got {other:?}"),
    }
}

async fn huggingface_config_metadata_case() {
    let fixture = Fixture::new(r#"{}"#);
    let transport = ScriptedTransport::new(vec![
        HttpReply::Status {
            status: 200,
            body: r#"{"max_position_embeddings": 4096}"#.to_owned(),
        },
        HttpReply::Status {
            status: 200,
            body: r#"{}"#.to_owned(),
        },
        HttpReply::Status {
            status: 404,
            body: r#"{"max_position_embeddings": 1}"#.to_owned(),
        },
        HttpReply::Status {
            status: 200,
            body: String::from("not json"),
        },
        HttpReply::Failure,
    ]);
    let hf = HfConfigClient {
        client: &transport,
        base_url: "https://huggingface.co".to_owned(),
    };
    for expected in [Some(4096), None, None, None, None] {
        let environment = Environment::new(&fixture);
        let mut keys = ModelCostKeys::default();
        let value = get_model_info_helper(
            &request("org/model", Some("huggingface")),
            &environment.metadata(),
            &mut keys,
            &NeverDynamic,
            &hf,
        )
        .await
        .unwrap();
        let value: Value = serde_json::from_str(value.get()).unwrap();
        assert_eq!(value["key"], "org/model");
        assert_eq!(value["litellm_provider"], "huggingface");
        assert_eq!(value["mode"], "chat");
        assert_eq!(value["input_cost_per_token"], 0);
        assert_eq!(value["output_cost_per_token"], 0);
        assert_eq!(
            value["max_tokens"],
            expected.map_or(Value::Null, Value::from)
        );
        for name in [
            "max_input_tokens",
            "max_output_tokens",
            "supports_system_messages",
            "supports_response_schema",
            "supports_function_calling",
            "supports_tool_choice",
            "supports_assistant_prefill",
            "supports_prompt_caching",
            "supports_prompt_cache_breakpoint",
            "supports_thinking_cache_preservation",
            "supports_computer_use",
            "supports_pdf_input",
        ] {
            assert_eq!(value[name], Value::Null, "{name}");
        }
    }
    assert_eq!(transport.seen().len(), 5);
    assert_eq!(
        transport.seen()[0].url,
        "https://huggingface.co/org/model/raw/main/config.json"
    );
    assert_eq!(transport.seen()[0].timeout, Duration::from_secs_f64(10.0));
}

#[test]
fn cost_map_limits_and_vertex_membership_follow_the_source() {
    assert!(ModelCostMap::parse(b"[]").is_err());
    let oversized = vec![b' '; 16 * 1024 * 1024 + 1];
    assert!(matches!(
        ModelCostMap::parse(&oversized),
        Err(ModelCostMapError::TooLarge { .. })
    ));
    let map = ModelCostMap::parse(
        br#"{
            "vertex_ai/meta/llama3-8b-instruct-maas": {"litellm_provider": "vertex_ai-llama_models"},
            "vertex_ai/codestral@latest": {"litellm_provider": "vertex_ai-mistral_models"},
            "vertex_ai/jamba-1.5@latest": {"litellm_provider": "vertex_ai-ai21_models"},
            "vertex_ai/other": {"litellm_provider": "vertex_ai-chat-models"}
        }"#,
    )
    .unwrap();
    assert!(map.vertex.llama.contains("meta/llama3-8b-instruct-maas"));
    assert!(map.vertex.mistral.contains("codestral@latest"));
    assert!(map.vertex.ai21.contains("jamba-1.5@latest"));
    assert_eq!(map.vertex.llama.len(), 1);
    assert!(VertexModelSets::default().llama.is_empty());
}

#[test]
fn decode_rejects_wrong_shaped_configs() {
    assert_eq!(
        decode_hf_model_config(br#"{"max_position_embeddings": 8}"#).unwrap(),
        Some(8)
    );
    assert_eq!(decode_hf_model_config(br#"{}"#).unwrap(), None);
    assert_eq!(
        decode_hf_model_config(br#"{"max_position_embeddings": null}"#).unwrap(),
        None
    );
    assert!(decode_hf_model_config(br#"{"max_position_embeddings": "8"}"#).is_err());
    assert!(decode_hf_model_config(br#"{"max_position_embeddings": 8.5}"#).is_err());
    assert!(decode_hf_model_config(b"[]").is_err());
    assert!(decode_hf_model_config(b"").is_err());
}

#[test]
fn the_injected_mantle_split_reads_the_region() {
    assert_eq!(
        test_mantle_region_split("us-east-1/model"),
        (Some("us-east-1"), "model")
    );
    assert_eq!(
        test_mantle_region_split("eu-west-1/model"),
        (None, "eu-west-1/model")
    );
    assert_eq!(test_mantle_region_split("model"), (None, "model"));
}
