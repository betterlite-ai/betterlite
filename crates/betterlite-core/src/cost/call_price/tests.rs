#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr as _;
use std::sync::{Mutex, PoisonError};

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::usage_sum::{PromptTokensDetails, Usage, UsageNumber};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use rust_decimal::Decimal;

use super::error::CallPriceError;
use super::policy::error::PolicyError;
use super::policy::{
    CharacterPriceRequest, ProviderPricePolicy, RerankPriceRequest, SearchPriceRequest,
    TokenPriceRequest, VectorStorePriceRequest, VertexRoute,
};
use super::usage::cache_details;
use super::{CallPriceEnv, CallPriceInput, CustomCostPerToken, call_price};
use crate::routes::CallType;

mod dispatch;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).unwrap()
}

fn record(json: &str) -> PriceRecord {
    serde_json::from_str(json).unwrap()
}

fn costs(result: GenericCostResult) -> (Decimal, Decimal) {
    (result.prompt_cost, result.completion_cost)
}

fn flat(input: &str, output: &str) -> CustomCostPerToken {
    CustomCostPerToken {
        input_cost_per_token: dec(input),
        output_cost_per_token: dec(output),
        cache_read_input_token_cost: None,
        cache_creation_cost: None,
    }
}

fn input(model: &str) -> CallPriceInput<'_> {
    CallPriceInput {
        model: Some(model),
        prompt_tokens: 0,
        completion_tokens: 0,
        response_time_ms: None,
        custom_llm_provider: None,
        region_name: None,
        prompt_characters: None,
        completion_characters: None,
        cache_creation_input_tokens: None,
        cache_read_input_tokens: None,
        custom_cost_per_token: None,
        custom_cost_per_second: None,
        number_of_queries: None,
        usage: None,
        rerank_billed_units: None,
        call_type: CallType::Completion,
        audio_seconds: Decimal::ZERO,
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        vector_response: None,
        ocr_billing: None,
        search_hidden_params: None,
        request_model: None,
        custom_model_info: None,
        server_tool_projection: None,
    }
}

struct Catalog {
    entries: BTreeMap<String, PriceRecord>,
    seen: Mutex<Vec<String>>,
}

impl Catalog {
    fn look(&self, kind: &str, model: &str, provider: Option<&str>) -> Option<&PriceRecord> {
        let line = format!("{kind} {model} {}", provider.unwrap_or("-"));
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
        self.entries.get(model)
    }
}

impl PriceLookup for Catalog {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        self.look("info", model, custom_llm_provider).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        self.look("priced", model, custom_llm_provider)
            .map(|found| PricedModel {
                record: found.clone(),
                tiered_pricing: Vec::new(),
                off_peak_pricing: None,
                off_peak_cache_read_present: false,
            })
    }
}

const FAKE_PROMPT: &str = "7";
const FAKE_COMPLETION: &str = "9";

type Priced = Result<GenericCostResult, PolicyError>;

struct Fake {
    seen: Mutex<Vec<String>>,
    route: VertexRoute,
    lyria: Option<Decimal>,
    vector: Option<GenericCostResult>,
}

impl Fake {
    fn log(&self, line: String) -> GenericCostResult {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line);
        GenericCostResult {
            prompt_cost: dec(FAKE_PROMPT),
            completion_cost: dec(FAKE_COMPLETION),
        }
    }
}

impl ProviderPricePolicy for Fake {
    fn vertex_route(&self, model: &str, call_type: CallType) -> Result<VertexRoute, PolicyError> {
        self.log(format!("route {model} {call_type:?}"));
        Ok(self.route)
    }

    fn lyria_cost(&self, provider: &str, model: &str) -> Result<Option<Decimal>, PolicyError> {
        self.log(format!("lyria {provider} {model}"));
        Ok(self.lyria)
    }

    fn token_cost(&self, request: &TokenPriceRequest<'_>) -> Priced {
        Ok(self.log(format!(
            "token {} {} {:?} {:?}",
            request.provider, request.model, request.service_tier, request.vertex_location
        )))
    }

    fn character_cost(&self, request: &CharacterPriceRequest<'_>) -> Priced {
        Ok(self.log(format!(
            "character {} {} {:?} {:?} {:?} {:?}",
            request.provider,
            request.model,
            request.prompt_characters,
            request.completion_characters,
            request.service_tier,
            request.vertex_location
        )))
    }

    fn search_cost(&self, request: &SearchPriceRequest<'_>) -> Priced {
        let queries = request.number_of_queries;
        let hidden = request.hidden_params.is_some();
        Ok(self.log(format!(
            "search {} {} {queries} {hidden}",
            request.provider, request.model
        )))
    }

    fn rerank_cost(&self, request: &RerankPriceRequest<'_>) -> Priced {
        let units = request.billed_units.is_some();
        let info = request.model_info.is_some();
        Ok(self.log(format!(
            "rerank {} {} {units} {info}",
            request.provider, request.model
        )))
    }

    fn vector_store_cost(
        &self,
        request: &VectorStorePriceRequest<'_>,
    ) -> Result<Option<GenericCostResult>, PolicyError> {
        self.log(format!(
            "vector {} {:?}",
            request.provider, request.api_type
        ));
        Ok(self.vector)
    }
}

struct Rig {
    catalog: Catalog,
    fake: Fake,
    catalog_model: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

fn rig(entries: &[(&str, &str)]) -> Rig {
    Rig {
        catalog: Catalog {
            entries: entries
                .iter()
                .map(|(key, json)| ((*key).to_owned(), record(json)))
                .collect(),
            seen: Mutex::new(Vec::new()),
        },
        fake: Fake {
            seen: Mutex::new(Vec::new()),
            route: VertexRoute::Token,
            lyria: None,
            vector: None,
        },
        catalog_model: ModelCatalog::default(),
        names: BTreeSet::new(),
        environment: BTreeMap::new(),
    }
}

impl Rig {
    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog_model,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }

    fn price(&self, request: &CallPriceInput<'_>) -> Result<GenericCostResult, CallPriceError> {
        let context = self.context();
        call_price(
            request,
            &CallPriceEnv {
                lookup: &self.catalog,
                policy: &self.fake,
                resolution: &context,
                database: &TimeZoneDatabase::from_env(),
                moment: DateTime::<Utc>::UNIX_EPOCH,
            },
        )
    }

    fn policy_calls(&self) -> Vec<String> {
        self.fake.seen.lock().unwrap().clone()
    }

    fn catalog_calls(&self) -> Vec<String> {
        self.catalog.seen.lock().unwrap().clone()
    }
}

fn modality_details(audio: Option<i64>, text: Option<i64>) -> PromptTokensDetails {
    PromptTokensDetails {
        text_tokens: text,
        audio_tokens: audio,
        image_tokens: None,
        video_tokens: None,
        web_search_requests: None,
        google_maps_grounding_requests: None,
        tool_use_tokens: None,
        character_count: None,
        image_count: None,
        video_length_seconds: None,
        audio_length_seconds: None,
        query_count: None,
        cache_write_tokens: None,
        cache_creation_tokens: None,
        cached_tokens: None,
        cached_tokens_details: None,
    }
}

const PRICED: &str = r#"{"input_cost_per_token": 0.001, "output_cost_per_token": 0.002}"#;

fn fake_pair() -> (Decimal, Decimal) {
    (dec(FAKE_PROMPT), dec(FAKE_COMPLETION))
}

struct Cache {
    cached: Option<i64>,
    write: Option<i64>,
    creation: Option<i64>,
    top_read: Option<i64>,
    top_creation: Option<i64>,
    explicit_read: Option<i64>,
    explicit_creation: Option<i64>,
}

impl Cache {
    fn usage(&self) -> Usage {
        let mut numeric_extras = BTreeMap::new();
        if let Some(count) = self.top_read {
            numeric_extras.insert(
                "cache_read_input_tokens".to_owned(),
                UsageNumber::Integer(count),
            );
        }
        if let Some(count) = self.top_creation {
            numeric_extras.insert(
                "cache_creation_input_tokens".to_owned(),
                UsageNumber::Integer(count),
            );
        }
        Usage {
            prompt_tokens: 999,
            completion_tokens: 999,
            total_tokens: 1998,
            prompt_tokens_details: Some(cache_details(self.cached, self.creation, self.write)),
            completion_tokens_details: None,
            numeric_extras,
        }
    }
}

#[test]
fn custom_prices_bill_the_explicit_tokens_and_not_the_usage_total() {
    let rig = rig(&[]);
    let usage = Usage {
        prompt_tokens: 999,
        completion_tokens: 999,
        total_tokens: 1998,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    };
    let request = CallPriceInput {
        prompt_tokens: 10,
        completion_tokens: 5,
        custom_cost_per_token: Some(flat("1", "2")),
        usage: Some(&usage),
        ..input("any-model")
    };
    let priced = rig.price(&request).unwrap();
    assert_eq!(costs(priced), (dec("10"), dec("10")));
    assert!(rig.catalog_calls().is_empty());
    assert!(rig.policy_calls().is_empty());
}

#[rstest]
#[case::details_only(Cache { cached: Some(3), write: Some(2), creation: None, top_read: None, top_creation: None, explicit_read: None, explicit_creation: None }, "12.5")]
#[case::creation_when_write_is_zero(Cache { cached: None, write: Some(0), creation: Some(4), top_read: None, top_creation: None, explicit_read: None, explicit_creation: None }, "18")]
#[case::write_wins_over_creation(Cache { cached: None, write: Some(2), creation: Some(4), top_read: None, top_creation: None, explicit_read: None, explicit_creation: None }, "14")]
#[case::top_level_zero_marks_anthropic_style(Cache { cached: Some(3), write: None, creation: None, top_read: None, top_creation: Some(0), explicit_read: None, explicit_creation: None }, "11.5")]
#[case::top_level_read_replaces_details(Cache { cached: Some(3), write: None, creation: None, top_read: Some(5), top_creation: None, explicit_read: None, explicit_creation: None }, "12.5")]
#[case::top_level_creation_adds_to_prompt(Cache { cached: None, write: None, creation: None, top_read: None, top_creation: Some(2), explicit_read: None, explicit_creation: None }, "16")]
#[case::falsey_detail_falls_back_to_explicit_read(Cache { cached: Some(0), write: None, creation: None, top_read: None, top_creation: None, explicit_read: Some(4), explicit_creation: None }, "12")]
#[case::explicit_creation_without_details(Cache { cached: None, write: None, creation: None, top_read: None, top_creation: None, explicit_read: None, explicit_creation: Some(2) }, "16")]
#[case::truthy_detail_ignores_explicit_read(Cache { cached: Some(3), write: None, creation: None, top_read: None, top_creation: None, explicit_read: Some(9), explicit_creation: None }, "8.5")]
#[case::truthy_creation_ignores_explicit_creation(Cache { cached: None, write: None, creation: Some(2), top_read: None, top_creation: None, explicit_read: None, explicit_creation: Some(9) }, "14")]
fn custom_prices_normalize_the_cache_counts(#[case] cache: Cache, #[case] expected: &str) {
    let usage = cache.usage();
    let request = CallPriceInput {
        prompt_tokens: 10,
        custom_cost_per_token: Some(CustomCostPerToken {
            cache_read_input_token_cost: Some(dec("0.5")),
            cache_creation_cost: Some(dec("3")),
            ..flat("1", "2")
        }),
        usage: Some(&usage),
        cache_read_input_tokens: cache.explicit_read,
        cache_creation_input_tokens: cache.explicit_creation,
        ..input("any-model")
    };
    assert_eq!(
        costs(rig(&[]).price(&request).unwrap()),
        (dec(expected), Decimal::ZERO)
    );
}

#[test]
fn a_per_second_custom_price_bills_the_response_time() {
    let request = CallPriceInput {
        response_time_ms: Some(dec("2000")),
        custom_cost_per_second: Some(dec("3")),
        ..input("any-model")
    };
    assert_eq!(
        costs(rig(&[]).price(&request).unwrap()),
        (dec("0"), dec("6"))
    );
}

#[test]
fn a_null_model_is_rejected_before_a_free_custom_price_and_an_empty_one_is_not() {
    let free = Some(flat("0", "0"));
    let null = CallPriceInput {
        model: None,
        custom_cost_per_token: free,
        ..input("")
    };
    let error = rig(&[]).price(&null).unwrap_err();
    assert!(matches!(error, CallPriceError::NullModel));
    assert_eq!(error.to_string(), "Invalid arg. Model cannot be none.");
    let empty = CallPriceInput {
        custom_cost_per_token: free,
        ..input("")
    };
    assert_eq!(costs(rig(&[]).price(&empty).unwrap()), (dec("0"), dec("0")));
}

#[rstest]
#[case::fraction(UsageNumber::Decimal(dec("1.5")))]
#[case::boolean(UsageNumber::Boolean(true))]
fn a_non_integer_top_level_cache_count_names_its_field(#[case] value: UsageNumber) {
    let usage = Usage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::from([("cache_read_input_tokens".to_owned(), value)]),
    };
    let request = CallPriceInput {
        usage: Some(&usage),
        custom_cost_per_token: Some(flat("1", "2")),
        ..input("any-model")
    };
    let error = rig(&[]).price(&request).unwrap_err();
    assert!(matches!(
        error,
        CallPriceError::NonIntegerCount {
            field: "cache_read_input_tokens"
        }
    ));
}

#[test]
fn a_whole_decimal_top_level_cache_count_is_read_as_an_integer() {
    let usage = Usage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::from([(
            "cache_read_input_tokens".to_owned(),
            UsageNumber::Decimal(dec("4.0")),
        )]),
    };
    let request = CallPriceInput {
        prompt_tokens: 10,
        usage: Some(&usage),
        custom_cost_per_token: Some(flat("1", "2")),
        ..input("any-model")
    };
    assert_eq!(
        costs(rig(&[]).price(&request).unwrap()),
        (dec("14"), Decimal::ZERO)
    );
}

#[test]
fn counter_overflow_is_a_typed_failure() {
    let added = CallPriceInput {
        prompt_tokens: i64::MAX,
        cache_read_input_tokens: Some(1),
        custom_cost_per_token: Some(flat("1", "2")),
        ..input("any-model")
    };
    assert!(matches!(
        rig(&[]).price(&added).unwrap_err(),
        CallPriceError::CounterOverflow
    ));
    let total = CallPriceInput {
        custom_llm_provider: Some("openai"),
        prompt_tokens: i64::MAX,
        completion_tokens: 1,
        ..input("any-model")
    };
    assert!(matches!(
        rig(&[("any-model", "{}")]).price(&total).unwrap_err(),
        CallPriceError::CounterOverflow
    ));
}

#[rstest]
#[case::joined_first("chat", Some("deepseek"), None, &["deepseek/chat", "chat"], "deepseek/chat")]
#[case::current_when_joined_is_absent("chat", Some("deepseek"), None, &["chat"], "chat")]
#[case::repeated_prefixes_collapse("deepseek/deepseek/chat", Some("deepseek"), None, &["deepseek/chat"], "deepseek/chat")]
#[case::region_entry_wins("deepseek/chat", Some("deepseek"), Some("eu"), &["deepseek/eu/chat", "deepseek/chat"], "deepseek/eu/chat")]
#[case::region_joins_a_bare_model("chat", Some("deepseek"), Some("eu"), &["deepseek/eu/chat"], "deepseek/eu/chat")]
#[case::unregistered_region_is_ignored("deepseek/chat", Some("deepseek"), Some("eu"), &["deepseek/chat"], "deepseek/chat")]
#[case::stripped_model_when_nothing_else_matches("x/chat", Some("deepseek"), None, &["chat"], "chat")]
#[case::inferred_provider_leaves_the_name_unjoined("deepseek/chat", None, None, &["deepseek/chat", "chat"], "deepseek/chat")]
#[case::inferred_provider_falls_back_to_the_stripped_name("deepseek/chat", None, None, &["chat"], "chat")]
#[case::unmatched_name_stays_as_given("deepseek/chat", None, None, &[], "deepseek/chat")]
fn the_candidate_model_follows_the_catalog_order(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] region: Option<&str>,
    #[case] keys: &[&str],
    #[case] expected: &str,
) {
    let entries: Vec<(&str, &str)> = keys.iter().map(|key| (*key, "{}")).collect();
    let rig = rig(&entries);
    let request = CallPriceInput {
        custom_llm_provider: provider,
        region_name: region,
        ..input(model)
    };
    assert_eq!(costs(rig.price(&request).unwrap()), fake_pair());
    assert_eq!(
        rig.policy_calls(),
        vec![format!("token deepseek {expected} None None")]
    );
}

#[rstest]
#[case::databricks("databricks", Some("flex"))]
#[case::gemini("gemini", Some("flex"))]
#[case::fireworks("fireworks_ai", None)]
#[case::deepseek("deepseek", None)]
#[case::tencent("tencent", None)]
#[case::perplexity("perplexity", None)]
#[case::xai("xai", None)]
#[case::lemonade("lemonade", None)]
#[case::dashscope("dashscope", None)]
#[case::qwencloud("qwencloud", None)]
#[case::qwen_ai_platform("qwen_ai_platform", None)]
fn adapter_priced_providers_receive_the_tier_only_where_litellm_passes_it(
    #[case] provider: &str,
    #[case] tier: Option<&str>,
) {
    let rig = rig(&[]);
    let request = CallPriceInput {
        custom_llm_provider: Some(provider),
        service_tier: Some("flex"),
        ..input("model-a")
    };
    assert_eq!(costs(rig.price(&request).unwrap()), fake_pair());
    assert_eq!(
        rig.policy_calls(),
        vec![format!("token {provider} model-a {tier:?} None")]
    );
}

#[rstest]
#[case::token(
    VertexRoute::Token,
    "token vertex_ai gemini Some(\"flex\") Some(\"us\")"
)]
#[case::character(
    VertexRoute::Character,
    "character vertex_ai gemini Some(4) Some(5) Some(\"flex\") Some(\"us\")"
)]
fn vertex_prices_the_prefix_stripped_model_by_the_route(
    #[case] route: VertexRoute,
    #[case] expected: &str,
) {
    let mut rig = rig(&[]);
    rig.fake.route = route;
    let request = CallPriceInput {
        custom_llm_provider: Some("vertex_ai"),
        prompt_characters: Some(4),
        completion_characters: Some(5),
        service_tier: Some("flex"),
        vertex_location: Some("us"),
        ..input("x/gemini")
    };
    assert_eq!(costs(rig.price(&request).unwrap()), fake_pair());
    assert_eq!(
        rig.policy_calls(),
        vec!["route gemini Completion".to_owned(), expected.to_owned()]
    );
}

#[rstest]
#[case::openai("openai")]
#[case::azure("azure")]
#[case::anthropic("anthropic")]
#[case::bedrock("bedrock")]
#[case::azure_ai("azure_ai")]
fn native_providers_bill_the_catalog_token_rates(#[case] provider: &str) {
    let rig = rig(&[("model-a", PRICED)]);
    let request = CallPriceInput {
        custom_llm_provider: Some(provider),
        prompt_tokens: 10,
        completion_tokens: 5,
        ..input("model-a")
    };
    assert_eq!(
        costs(rig.price(&request).unwrap()),
        (dec("0.01"), dec("0.01"))
    );
    assert!(rig.policy_calls().is_empty());
}

#[rstest]
#[case::openai("openai")]
#[case::azure("azure")]
fn generic_providers_look_up_the_candidate_under_their_own_name(#[case] provider: &str) {
    let rig = rig(&[("model-a", PRICED)]);
    let request = CallPriceInput {
        custom_llm_provider: Some(provider),
        ..input("model-a")
    };
    rig.price(&request).unwrap();
    assert_eq!(
        rig.catalog_calls(),
        vec![
            format!("info model-a {provider}"),
            format!("priced model-a {provider}")
        ]
    );
}

#[test]
fn a_supplied_usage_is_priced_instead_of_the_explicit_counters() {
    let rig = rig(&[("model-a", PRICED)]);
    let usage = Usage {
        prompt_tokens: 100,
        completion_tokens: 50,
        total_tokens: 150,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    };
    let request = CallPriceInput {
        custom_llm_provider: Some("openai"),
        prompt_tokens: 10,
        usage: Some(&usage),
        ..input("model-a")
    };
    assert_eq!(
        costs(rig.price(&request).unwrap()),
        (dec("0.1"), dec("0.1"))
    );
}

#[test]
fn explicit_cache_counters_become_cached_prompt_tokens() {
    let entry = r#"{"input_cost_per_token": 0.001, "output_cost_per_token": 0.002, "cache_read_input_token_cost": 0.0005}"#;
    let rig = rig(&[("model-a", entry)]);
    let request = CallPriceInput {
        custom_llm_provider: Some("openai"),
        prompt_tokens: 10,
        cache_read_input_tokens: Some(4),
        ..input("model-a")
    };
    assert_eq!(
        costs(rig.price(&request).unwrap()),
        (dec("0.008"), dec("0"))
    );
}

#[test]
fn explicit_cache_creation_counter_is_billed_at_the_creation_rate() {
    let entry = r#"{"input_cost_per_token": 0.001, "output_cost_per_token": 0.002, "cache_creation_input_token_cost": 0.002}"#;
    let rig = rig(&[("model-a", entry)]);
    let request = CallPriceInput {
        custom_llm_provider: Some("openai"),
        prompt_tokens: 10,
        cache_creation_input_tokens: Some(4),
        ..input("model-a")
    };
    assert_eq!(
        costs(rig.price(&request).unwrap()),
        (dec("0.014"), dec("0"))
    );
}

#[rstest]
#[case::anthropic("anthropic", vec!["info model-a anthropic", "priced model-a anthropic", "info model-a anthropic"])]
#[case::bedrock("bedrock", vec!["info model-a bedrock", "priced model-a bedrock"])]
#[case::azure_ai("azure_ai", vec!["info model-a azure_ai", "priced model-a azure_ai"])]
fn native_providers_skip_the_declared_rate_probe(
    #[case] provider: &str,
    #[case] expected: Vec<&str>,
) {
    let rig = rig(&[("model-a", PRICED)]);
    let request = CallPriceInput {
        custom_llm_provider: Some(provider),
        prompt_tokens: 10,
        ..input("model-a")
    };
    rig.price(&request).unwrap();
    assert_eq!(rig.catalog_calls(), expected);
}

#[rstest]
#[case::input_rate(r#"{"input_cost_per_token": 0.001, "output_cost_per_token": 0}"#, true)]
#[case::output_rate_only(r#"{"input_cost_per_token": 0, "output_cost_per_token": 0.002}"#, true)]
#[case::tiered_pricing(
    r#"{"input_cost_per_token": 0, "output_cost_per_token": 0, "tiered_pricing": []}"#,
    true
)]
#[case::null_tiered_pricing(
    r#"{"input_cost_per_token": 0, "output_cost_per_token": 0, "tiered_pricing": null}"#,
    false
)]
#[case::no_rates("{}", false)]
fn other_providers_bill_tokens_only_when_the_catalog_declares_rates(
    #[case] entry: &str,
    #[case] billed: bool,
) {
    let rig = rig(&[("model-a", entry)]);
    let request = CallPriceInput {
        custom_llm_provider: Some("mystery"),
        prompt_tokens: 10,
        completion_tokens: 5,
        ..input("model-a")
    };
    let priced = rig.price(&request).unwrap();
    assert_eq!(
        rig.catalog_calls()
            .contains(&"priced model-a mystery".to_owned()),
        billed
    );
    if !billed {
        assert_eq!(costs(priced), (dec("0"), dec("0")));
    }
}

#[test]
fn an_unknown_model_under_another_provider_fails() {
    let request = CallPriceInput {
        custom_llm_provider: Some("mystery"),
        ..input("model-a")
    };
    let error = rig(&[]).price(&request).unwrap_err();
    assert!(matches!(error, CallPriceError::Duration(_)));
}

#[test]
fn wall_clock_pricing_comes_before_the_provider_matrix() {
    let rig = rig(&[("model-a", r#"{"cost_per_second": 0.5}"#)]);
    let request = CallPriceInput {
        custom_llm_provider: Some("deepseek"),
        response_time_ms: Some(dec("4000")),
        ..input("model-a")
    };
    assert_eq!(costs(rig.price(&request).unwrap()), (dec("2"), dec("0")));
    assert!(rig.policy_calls().is_empty());
}
