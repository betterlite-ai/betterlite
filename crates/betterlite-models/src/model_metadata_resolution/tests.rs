#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::convert::Infallible;
use std::future::{Future, ready};
use std::io;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;

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

type Fields = BTreeMap<String, Box<RawValue>>;

mod dynamic;
mod ordered;

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
    model
        .strip_prefix("converse/")
        .map_or_else(|| model.to_owned(), str::to_owned)
}

fn test_mantle_region_split(model: &str) -> (Option<&str>, &str) {
    match model.split_once('/') {
        Some((head, rest)) if head == "us-east-1" => (Some(head), rest),
        _ => (None, model),
    }
}

fn raw(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.to_owned()).expect("raw JSON")
}

fn fields(raw: &RawValue) -> Fields {
    serde_json::from_str(raw.get()).expect("JSON object")
}

fn string_field(fields: &Fields, name: &str) -> String {
    let value = fields.get(name).expect(name);
    serde_json::from_str(value.get()).expect(name)
}

fn number_field(fields: &Fields, name: &str) -> Option<i64> {
    let value = fields.get(name)?;
    serde_json::from_str(value.get()).ok()
}

fn bool_field(fields: &Fields, name: &str) -> Option<bool> {
    let value = fields.get(name)?;
    serde_json::from_str(value.get()).ok()
}

fn raw_field(fields: &Fields, name: &str) -> String {
    fields
        .get(name)
        .map(|value| value.get())
        .unwrap_or_default()
        .to_owned()
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
        Self {
            map,
            catalog,
            environment: BTreeMap::new(),
            known_model_names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            rules: Vec::new(),
        }
    }

    fn with_rules(raw_map: &str, catalog: &str, rules: &str) -> Self {
        let mut fixture = Self::new(catalog);
        fixture.map = ModelCostMap::parse(raw_map.as_bytes()).expect("model cost map");
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
        Self {
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

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("no Hugging Face config transport is configured")]
struct NoConfig;

impl ModelInfoClient for NoConfig {
    type Error = Self;

    fn get(
        &self,
        _request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        ready(Err(Self))
    }
}

async fn resolve<Source>(
    fixture: &Fixture,
    model: &str,
    provider: Option<&str>,
    dynamic: &Source,
) -> Result<Fields, ModelMetadataError>
where
    Source: DynamicMetadataSource,
{
    let environment = Environment::new(fixture);
    let hf = HfConfigClient {
        client: NoConfig,
        base_url: String::new(),
    };
    let mut keys = ModelCostKeys::default();
    let answer = get_model_info_helper(
        &request(model, provider),
        &environment.metadata(),
        &mut keys,
        dynamic,
        &hf,
    )
    .await;
    answer.map(|raw| fields(&raw))
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
        ready(Ok(None))
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
    Value(Box<RawValue>),
    None,
    Error,
}

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
                api_key: api_key.map(|key| key.expose_secret().to_owned()),
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
            Reply::Error => Err(io::Error::other("dynamic metadata failed")),
        })
    }
}

#[derive(Clone, Debug)]
enum HttpReply {
    Status { status: u16, body: String },
    Failure,
}

#[derive(Default)]
struct ScriptedTransport {
    replies: Mutex<VecDeque<HttpReply>>,
    seen: Mutex<Vec<ModelInfoRequest>>,
}

impl ScriptedTransport {
    fn new(replies: Vec<HttpReply>) -> Self {
        Self {
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
        ready(match reply {
            HttpReply::Status { status, body } => {
                Ok(FetchResponse::new(status, Vec::new(), body.into_bytes()))
            }
            HttpReply::Failure => Err(io::Error::other("config fetch failed")),
        })
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
        Scenario::PotentialNames => {
            potential_names_case();
            explicit_potential_names_case();
        }
        Scenario::ProviderAliases => provider_aliases_case().await,
        Scenario::DynamicProviderMetadata => dynamic::dynamic_provider_metadata_case().await,
        Scenario::OrderedExactLookup => ordered::ordered_exact_lookup_case().await,
        Scenario::FillMissingExactEntry => fill_missing_exact_entry_case().await,
        Scenario::GeneralizationOnlyUnknown => generalization_only_unknown_case().await,
        Scenario::UnmappedErrorBoundary => unmapped_error_boundary_case().await,
        Scenario::HuggingfaceConfigMetadata => huggingface_config_metadata_case().await,
    }
}

fn potential_names_case() {
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
    assert_eq!(inferred.combined_stripped_model_name, "gpt-4o");
    assert_eq!(inferred.provider_prefixed_model_name, "gpt-4o");

    let unknown = names(&fixture, "not-anywhere-xyz", None);
    assert_eq!(unknown.custom_llm_provider, None);
    assert_eq!(unknown.split_model, "not-anywhere-xyz");
    assert_eq!(unknown.combined_model_name, "not-anywhere-xyz");
    assert_eq!(unknown.provider_prefixed_model_name, "not-anywhere-xyz");

    let vertex = names(&fixture, "vertex-model", None);
    assert_eq!(vertex.custom_llm_provider.as_deref(), Some("vertex_ai"));

    let routed = Fixture::new(
        r#"{
            "fallback_generalizations": {"rules": [
                {"pattern": "^brand-new-", "model_info": {"litellm_provider": "anthropic"}}
            ]}
        }"#,
    );
    let brand_new = names(&routed, "brand-new-model-7", None);
    assert_eq!(brand_new.custom_llm_provider.as_deref(), Some("anthropic"));
    assert_eq!(brand_new.split_model, "brand-new-model-7");
    assert_eq!(brand_new.provider_prefixed_model_name, "brand-new-model-7");
}

fn explicit_potential_names_case() {
    let fixture = Fixture::new("{}");
    let prefixed = names(&fixture, "groq/llama3-8b", Some("groq"));
    assert_eq!(prefixed.split_model, "llama3-8b");
    assert_eq!(prefixed.combined_model_name, "groq/llama3-8b");
    assert_eq!(prefixed.combined_stripped_model_name, "groq/llama3-8b");
    assert_eq!(prefixed.provider_prefixed_model_name, "groq/groq/llama3-8b");

    let dated = names(&fixture, "llama3-8b-2024-01-01", Some("groq"));
    assert_eq!(dated.combined_model_name, "groq/llama3-8b-2024-01-01");
    assert_eq!(dated.stripped_model_name, "llama3-8b");
    assert_eq!(dated.combined_stripped_model_name, "groq/llama3-8b");
    assert_eq!(
        dated.provider_prefixed_model_name,
        "groq/llama3-8b-2024-01-01"
    );
}

#[test]
fn bedrock_and_mantle_names_follow_the_source() {
    let fixture = Fixture::new("{}");
    let bedrock = names(&fixture, "converse/anthropic.claude-3", Some("bedrock"));
    assert_eq!(bedrock.stripped_model_name, "anthropic.claude-3");
    assert_eq!(
        bedrock.combined_model_name,
        "bedrock/converse/anthropic.claude-3"
    );
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
    azure_alias_case().await;
    vertex_alias_case().await;
    declared_provider_case().await;
}

async fn azure_alias_case() {
    let fixture = Fixture::new(
        r#"{
            "azure/gpt-35-turbo": {"litellm_provider": "azure", "mode": "chat", "input_cost_per_token": 1},
            "azure/ada": {"litellm_provider": "azure", "mode": "embedding", "input_cost_per_token": 2}
        }"#,
    );
    let azure = resolve(&fixture, "gpt-35-turbo", None, &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(string_field(&azure, "key"), "azure/gpt-35-turbo");
    let embedding = resolve(&fixture, "ada", None, &NeverDynamic).await.unwrap();
    assert_eq!(string_field(&embedding, "key"), "azure/ada");
}

async fn vertex_alias_case() {
    let fixture = Fixture::new(
        r#"{
            "vertex_ai/meta/llama3-8b-instruct-maas": {"litellm_provider": "vertex_ai-llama_models", "mode": "chat", "input_cost_per_token": 3},
            "vertex_ai/codestral@latest": {"litellm_provider": "vertex_ai-mistral_models", "mode": "chat", "input_cost_per_token": 4},
            "vertex_ai/jamba-1.5@latest": {"litellm_provider": "vertex_ai-ai21_models", "mode": "chat", "input_cost_per_token": 5}
        }"#,
    );
    let beta = resolve(
        &fixture,
        "llama3-8b-instruct-maas",
        Some("vertex_ai_beta"),
        &NeverDynamic,
    )
    .await
    .unwrap();
    assert_eq!(
        string_field(&beta, "key"),
        "vertex_ai/meta/llama3-8b-instruct-maas"
    );
    let mistral = resolve(&fixture, "codestral", Some("vertex_ai"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(string_field(&mistral, "key"), "vertex_ai/codestral@latest");
    let ai21 = resolve(&fixture, "jamba-1.5", Some("vertex_ai"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(string_field(&ai21, "key"), "vertex_ai/jamba-1.5@latest");
}

async fn declared_provider_case() {
    let fixture = Fixture::new(
        r#"{"github_copilot/gpt-4": {"litellm_provider": "github_copilot", "mode": "chat", "input_cost_per_token": 6}}"#,
    );
    let declared = resolve(&fixture, "github_copilot/gpt-4", None, &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(string_field(&declared, "key"), "github_copilot/gpt-4");
}

async fn fill_missing_exact_entry_case() {
    let fixture = fill_fixture();
    fill_modes_case(&fixture).await;
    fill_preserves_explicit_case(&fixture).await;
    fill_uses_entry_provider_case().await;
}

fn fill_fixture() -> Fixture {
    let rules = r#"[{
        "pattern": "^groq/",
        "fill_missing_for_providers": ["groq"],
        "model_info": {"max_tokens": 100, "supports_vision": true, "input_cost_per_token": 99, "mode": "completion"}
    }]"#;
    let entries = r#"{
        "groq/model": {"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 1},
        "groq/absent-mode": {"litellm_provider": "groq", "input_cost_per_token": 1},
        "groq/responses": {"litellm_provider": "groq", "mode": "responses", "input_cost_per_token": 1},
        "groq/completion": {"litellm_provider": "groq", "mode": "completion", "input_cost_per_token": 1},
        "groq/null-mode": {"litellm_provider": "groq", "mode": null, "input_cost_per_token": 1},
        "groq/explicit": {"litellm_provider": "groq", "mode": "chat", "max_tokens": null, "supports_vision": false, "rpm": 0, "input_cost_per_token": 1}
    }"#;
    Fixture::with_rules(entries, entries, rules)
}

async fn fill_modes_case(fixture: &Fixture) {
    let chat = resolve(fixture, "model", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(number_field(&chat, "max_tokens"), Some(100));
    assert_eq!(bool_field(&chat, "supports_vision"), Some(true));
    assert_eq!(number_field(&chat, "input_cost_per_token"), Some(1));
    assert_eq!(string_field(&chat, "mode"), "chat");

    let absent_mode = resolve(fixture, "absent-mode", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(number_field(&absent_mode, "max_tokens"), Some(100));
    let responses = resolve(fixture, "responses", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(number_field(&responses, "max_tokens"), Some(100));
    let completion = resolve(fixture, "completion", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(raw_field(&completion, "max_tokens"), "null");
    let null_mode = resolve(fixture, "null-mode", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(raw_field(&null_mode, "max_tokens"), "null");
}

async fn fill_preserves_explicit_case(fixture: &Fixture) {
    let explicit = resolve(fixture, "explicit", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(raw_field(&explicit, "max_tokens"), "null");
    assert_eq!(bool_field(&explicit, "supports_vision"), Some(false));
    assert_eq!(number_field(&explicit, "rpm"), Some(0));
    assert_eq!(
        fixture.map.entries["groq/model"].get(),
        r#"{"litellm_provider": "groq", "mode": "chat", "input_cost_per_token": 1}"#
    );
}

async fn fill_uses_entry_provider_case() {
    let rules = r#"[{
        "pattern": "^other/",
        "fill_missing_for_providers": ["groq"],
        "model_info": {"max_tokens": 5}
    },
    {
        "pattern": "^bare-",
        "fill_missing_for_providers": ["groq"],
        "model_info": {"max_tokens": 5}
    }]"#;
    let entries = r#"{
        "other/model": {"litellm_provider": "other", "mode": "chat"},
        "bare-model": {"mode": "chat"}
    }"#;
    let fixture = Fixture::with_rules(entries, entries, rules);
    let other = resolve(&fixture, "other/model", Some("other"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(raw_field(&other, "max_tokens"), "null");
    let bare = resolve(&fixture, "bare-model", Some("groq"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(raw_field(&bare, "max_tokens"), "null");
}

async fn generalization_only_unknown_case() {
    let fixture = generalization_fixture();
    generalization_rules_case(&fixture).await;
    generalization_empty_rule_case(&fixture).await;
    generalization_blocked_case(&fixture).await;
}

fn generalization_fixture() -> Fixture {
    let rules = r#"[
        {"pattern": "^anthropic/claude-future$", "model_info": {"max_tokens": 4096, "litellm_provider": "anthropic"}},
        {"pattern": "^claude-future$", "model_info": {"max_tokens": 1}},
        {"pattern": "^mystery-model$", "model_info": {"max_tokens": 7, "litellm_provider": "acme"}},
        {"pattern": "^empty-rule$", "model_info": {}},
        {"pattern": "^anthropic/blocked", "model_info": {"max_tokens": 11}}
    ]"#;
    Fixture::with_rules(
        r#"{"anthropic/blocked-prefix": {"litellm_provider": "openai", "mode": "chat"}}"#,
        r#"{"anthropic/blocked-prefix": {"litellm_provider": "openai", "mode": "chat"}}"#,
        rules,
    )
}

async fn generalization_rules_case(fixture: &Fixture) {
    let projected = resolve(fixture, "claude-future", Some("anthropic"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(string_field(&projected, "key"), "anthropic/claude-future");
    assert_eq!(number_field(&projected, "max_tokens"), Some(4096));
    assert_eq!(string_field(&projected, "litellm_provider"), "anthropic");

    let first_match = resolve(
        fixture,
        "claude-future-2024-01-01",
        Some("anthropic"),
        &NeverDynamic,
    )
    .await
    .unwrap();
    assert_eq!(string_field(&first_match, "key"), "anthropic/claude-future");
    assert_eq!(number_field(&first_match, "max_tokens"), Some(4096));

    let retained = resolve(fixture, "mystery-model-2024-01-01", None, &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(string_field(&retained, "key"), "mystery-model");
    assert_eq!(number_field(&retained, "max_tokens"), Some(7));
    assert_eq!(string_field(&retained, "litellm_provider"), "acme");

    let overwritten = resolve(
        fixture,
        "mystery-model-2024-01-01",
        Some("other"),
        &NeverDynamic,
    )
    .await
    .unwrap();
    assert_eq!(string_field(&overwritten, "key"), "mystery-model");
    assert_eq!(string_field(&overwritten, "litellm_provider"), "other");
}

async fn generalization_empty_rule_case(fixture: &Fixture) {
    let empty = resolve(fixture, "empty-rule", Some("acme"), &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(string_field(&empty, "litellm_provider"), "acme");
    assert_eq!(raw_field(&empty, "max_tokens"), "null");

    let providerless = resolve(fixture, "empty-rule", None, &NeverDynamic)
        .await
        .unwrap();
    assert_eq!(raw_field(&providerless, "litellm_provider"), "null");
}

async fn generalization_blocked_case(fixture: &Fixture) {
    let blocked = resolve(fixture, "blocked-prefix", Some("anthropic"), &NeverDynamic).await;
    assert!(matches!(blocked, Err(ModelMetadataError::NotMapped { .. })));
}

async fn unmapped_error_boundary_case() {
    let fixture = Fixture::new(r#"{"azure/gpt-35-turbo": {"litellm_provider": "azure"}}"#);
    let missing = resolve(&fixture, "nowhere-model", Some("groq"), &NeverDynamic).await;
    let Err(error @ ModelMetadataError::NotMapped { .. }) = missing else {
        panic!("expected a not-mapped error");
    };
    assert_eq!(
        error.to_string(),
        "This model isn't mapped yet. model=nowhere-model, custom_llm_provider=groq. Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json."
    );

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
    let Err(raised @ ModelMetadataError::Failed { .. }) = failure else {
        panic!("expected a generic failure");
    };
    assert_eq!(
        raised.to_string(),
        "This model isn't mapped yet. model=vertex_ai/bad-model, custom_llm_provider=vertex_ai. Add it here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json."
    );
}

async fn huggingface_config_metadata_case() {
    huggingface_fixed_fields_case().await;
    huggingface_fetch_outcomes_case().await;
}

async fn huggingface_fixed_fields_case() {
    let fixture = Fixture::new("{}");
    let transport = ScriptedTransport::new(vec![HttpReply::Status {
        status: 200,
        body: r#"{"max_position_embeddings": 4096}"#.to_owned(),
    }]);
    let hf = HfConfigClient {
        client: &transport,
        base_url: "https://huggingface.co".to_owned(),
    };
    let value = huggingface_answer(&fixture, &hf).await;
    assert_eq!(string_field(&value, "key"), "org/model");
    assert_eq!(string_field(&value, "litellm_provider"), "huggingface");
    assert_eq!(string_field(&value, "mode"), "chat");
    assert_eq!(number_field(&value, "input_cost_per_token"), Some(0));
    assert_eq!(number_field(&value, "output_cost_per_token"), Some(0));
    assert_eq!(number_field(&value, "max_tokens"), Some(4096));
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
        assert_eq!(raw_field(&value, name), "null", "{name}");
    }
    let request = transport.seen().into_iter().next().expect("request");
    assert_eq!(
        request.url,
        "https://huggingface.co/org/model/raw/main/config.json"
    );
    assert_eq!(request.timeout, Duration::from_secs(10));
}

async fn huggingface_fetch_outcomes_case() {
    let fixture = Fixture::new("{}");
    let transport = ScriptedTransport::new(vec![
        HttpReply::Status {
            status: 200,
            body: r#"{"max_position_embeddings": 4096}"#.to_owned(),
        },
        HttpReply::Status {
            status: 200,
            body: String::from("{}"),
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
        let value = huggingface_answer(&fixture, &hf).await;
        assert_eq!(number_field(&value, "max_tokens"), expected);
        assert_eq!(string_field(&value, "litellm_provider"), "huggingface");
    }
    assert_eq!(transport.seen().len(), 5);
}

async fn huggingface_answer<Transport>(fixture: &Fixture, hf: &HfConfigClient<Transport>) -> Fields
where
    Transport: ModelInfoClient,
{
    let environment = Environment::new(fixture);
    let mut keys = ModelCostKeys::default();
    get_model_info_helper(
        &request("org/model", Some("huggingface")),
        &environment.metadata(),
        &mut keys,
        &NeverDynamic,
        hf,
    )
    .await
    .map(|raw| fields(&raw))
    .unwrap()
}

#[test]
fn cost_map_limits_and_vertex_membership_follow_the_source() {
    assert!(ModelCostMap::parse(b"[]").err().is_some());
    let oversized = vec![b' '; 16 * 1024 * 1024 + 1];
    assert!(matches!(
        ModelCostMap::parse(&oversized),
        Err(ModelCostMapError::TooLarge { .. })
    ));
    let at_limit = vec![b' '; 16 * 1024 * 1024];
    assert!(matches!(
        ModelCostMap::parse(&at_limit),
        Err(ModelCostMapError::Json(_))
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
        decode_hf_model_config(br#"{"max_position_embeddings": 8}"#)
            .ok()
            .flatten(),
        Some(8)
    );
    assert_eq!(decode_hf_model_config(b"{}").ok().flatten(), None);
    assert!(matches!(
        decode_hf_model_config(br#"{"max_position_embeddings": null}"#),
        Ok(None)
    ));
    assert!(
        decode_hf_model_config(br#"{"max_position_embeddings": "8"}"#)
            .err()
            .is_some()
    );
    assert!(
        decode_hf_model_config(br#"{"max_position_embeddings": 8.5}"#)
            .err()
            .is_some()
    );
    assert!(decode_hf_model_config(b"[]").err().is_some());
    assert!(decode_hf_model_config(b"[4096]").err().is_some());
    assert!(decode_hf_model_config(b"").err().is_some());
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
