#![cfg(test)]

use std::collections::HashMap;
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::rstest;

use super::error::{EntryError, RegistrationError, RegistrationWarning};
use super::merge::{convert_stringified_numbers, update_dictionary};
use super::{
    CostMapFetcher, JsonObject, LiveDeploymentReplay, LiveRegistration, ModelCostEntries,
    ModelCostSource, ModelInfoLookup, ModelRegistry, RegisterOptions, Registered,
    RegistrationEnvironment,
};
use crate::catalog_generalizations::CatalogGeneralizations;
use crate::decisions::Json;
use crate::model_catalog_keys::ModelCostKeys;
use crate::model_registration::builtin::is_generalized_model_info;
use crate::provider_model_sets::{ProviderModelSet, ProviderModelSets};

const NO_RULES: &str = "[]";
const FT_RULES: &str = r#"[{"pattern":"^ft:","model_info":{"mode":"chat","max_tokens":4096}}]"#;
const PLAIN: RegisterOptions<'static> = RegisterOptions {
    persist_across_reloads: false,
    warning_display_name: None,
};

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

fn entries(text: &str) -> ModelCostEntries {
    serde_json::from_str(text).unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn live_registry(live: &str) -> ModelRegistry {
    ModelRegistry {
        model_cost: object(live),
        runtime_registered: ModelCostEntries::new(),
        provider_sets: ProviderModelSets::default(),
        keys: ModelCostKeys::default(),
        live_replay: None,
    }
}

fn warnings(models: &[&str]) -> Vec<RegistrationWarning> {
    models
        .iter()
        .map(|model| RegistrationWarning::MissingCachePricing {
            model: (*model).to_owned(),
        })
        .collect()
}

fn same_name(model: &str) -> String {
    model.to_owned()
}

struct Lookup {
    infos: ModelCostEntries,
    cleared: AtomicUsize,
}

impl ModelInfoLookup for Lookup {
    fn get_model_info(
        &self,
        model: &str,
        _model_cost: &JsonObject,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Option<JsonObject>> + Send {
        ready(self.infos.get(model).cloned())
    }

    fn clear_caches(&self) {
        self.cleared.fetch_add(1, Ordering::SeqCst);
    }
}

// The url and the attempt count come back inside the entry, so a test can see what was asked.
struct Fetcher {
    fails: bool,
}

impl CostMapFetcher for Fetcher {
    type Error = io::Error;

    fn get_model_cost_map(
        &self,
        url: &str,
        max_attempts: u32,
    ) -> impl Future<Output = Result<ModelCostEntries, io::Error>> + Send {
        ready(if self.fails {
            Err(io::Error::other("unreachable"))
        } else {
            Ok(entries(&format!(
                r#"{{"{url}":{{"litellm_provider":"openai","attempts":{max_attempts}}}}}"#
            )))
        })
    }
}

struct World {
    lookup: Lookup,
    fetcher: Fetcher,
    generalizations: CatalogGeneralizations,
}

impl World {
    fn new(infos: &str, rules: &str) -> Self {
        Self {
            lookup: Lookup {
                infos: entries(infos),
                cleared: AtomicUsize::new(0),
            },
            fetcher: Fetcher { fails: false },
            generalizations: CatalogGeneralizations::install(serde_json::from_str(rules).unwrap())
                .0,
        }
    }

    fn environment(&self) -> RegistrationEnvironment<'_, Lookup, Fetcher> {
        RegistrationEnvironment {
            lookup: &self.lookup,
            fetcher: &self.fetcher,
            generalizations: &self.generalizations,
            bedrock_base_model: same_name,
        }
    }

    fn cleared(&self) -> usize {
        self.lookup.cleared.load(Ordering::SeqCst)
    }
}

async fn try_register(
    registry: &mut ModelRegistry,
    world: &World,
    source: ModelCostSource,
    options: &RegisterOptions<'_>,
) -> Result<Registered, RegistrationError<io::Error>> {
    registry
        .register_model(source, options, &world.environment())
        .await
}

async fn register_entries(registry: &mut ModelRegistry, world: &World, value: &str) -> Registered {
    try_register(
        registry,
        world,
        ModelCostSource::Entries(entries(value)),
        &PLAIN,
    )
    .await
    .unwrap()
}

struct Replay(Vec<LiveRegistration>);

impl LiveDeploymentReplay for Replay {
    fn registrations(&self) -> Vec<LiveRegistration> {
        self.0.clone()
    }
}

fn live_registration(
    value: &str,
    persist_across_reloads: bool,
    display_name: Option<&str>,
) -> Replay {
    Replay(vec![LiveRegistration {
        entries: entries(value),
        persist_across_reloads,
        warning_display_name: display_name.map(str::to_owned),
    }])
}

#[rstest]
#[case::null_values_are_skipped(r#"{"a":1}"#, r#"{"a":null,"b":null}"#, r#"{"a":1}"#)]
#[case::text_becomes_a_number(
    "{}",
    r#"{"a":"3e-07","b":"8192","c":"abc"}"#,
    r#"{"a":3e-7,"b":8192,"c":"abc"}"#
)]
#[case::text_replaces_an_older_value(r#"{"a":"old"}"#, r#"{"a":"7"}"#, r#"{"a":7}"#)]
#[case::nested_objects_merge_one_level(
    r#"{"a":{"x":1,"y":{"p":1}}}"#,
    r#"{"a":{"y":{"q":2},"z":3}}"#,
    r#"{"a":{"x":1,"y":{"q":2},"z":3}}"#
)]
#[case::an_object_replaces_a_scalar(r#"{"a":5}"#, r#"{"a":{"x":1}}"#, r#"{"a":{"x":1}}"#)]
#[case::an_object_is_copied_when_absent("{}", r#"{"a":{"x":1}}"#, r#"{"a":{"x":1}}"#)]
#[case::nested_text_is_not_converted("{}", r#"{"a":{"x":"3e-07"}}"#, r#"{"a":{"x":"3e-07"}}"#)]
#[case::other_types_are_assigned(
    r#"{"a":1}"#,
    r#"{"a":false,"b":[1,"2"],"c":2.5}"#,
    r#"{"a":false,"b":[1,"2"],"c":2.5}"#
)]
fn update_dictionary_merge_cases(
    #[case] existing: &str,
    #[case] incoming: &str,
    #[case] expected: &str,
) {
    let mut merged = object(existing);
    update_dictionary(&mut merged, &object(incoming));
    assert_eq!(merged, object(expected));
}

#[rstest]
#[case::scientific("3e-07", "3e-7")]
#[case::whole("8192", "8192")]
#[case::negative_whole("-12", "-12")]
#[case::decimal("1.5", "1.5")]
#[case::trailing_zero("2.50", "2.5")]
#[case::leading_point(".5", "0.5")]
#[case::trailing_point("5.", "5.0")]
#[case::exponent_without_point("1e5", "100000.0")]
#[case::exponent_with_point("1.5e3", "1500.0")]
#[case::leading_zeros("007", "7")]
#[case::all_zeros("00", "0")]
#[case::negative_zero("-0", "0")]
#[case::explicit_plus("+5", "5")]
#[case::surrounding_spaces(" 12 ", "12")]
#[case::very_large_whole("123456789012345678901234567890", "123456789012345678901234567890")]
#[case::words_stay_text("abc", r#""abc""#)]
#[case::empty_stays_text("", r#""""#)]
#[case::bare_exponent_stays_text("e", r#""e""#)]
#[case::bare_sign_stays_text("-", r#""-""#)]
#[case::digits_with_letters_stay_text("12abc", r#""12abc""#)]
#[case::nan_stays_text("nan", r#""nan""#)]
#[case::overflowing_float_stays_text("1e999", r#""1e999""#)]
#[case::underscore_grouping_stays_text("1_000", r#""1_000""#)]
#[case::non_ascii_digits_stay_text("\u{0661}\u{0662}", r#""\u0661\u0662""#)]
fn convert_number_cases(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(convert_stringified_numbers(text), json(expected));
}

#[rstest]
#[case::entry_without_a_price(r#"{"a":{"mode":"chat"}}"#, &["a"], true)]
#[case::input_price(r#"{"a":{"input_cost_per_token":0}}"#, &["a"], false)]
#[case::output_price(r#"{"a":{"output_cost_per_token":0}}"#, &["a"], false)]
#[case::a_null_price_is_present(r#"{"a":{"output_cost_per_token":null}}"#, &["a"], false)]
#[case::one_priced_entry_is_enough(
    r#"{"a":{},"b":{"output_cost_per_token":1}}"#,
    &["a", "b"],
    false
)]
#[case::every_entry_unpriced(r#"{"a":{},"b":{}}"#, &["a", "b"], true)]
#[case::missing_entries_are_ignored(r#"{"a":{}}"#, &["missing", "a"], true)]
#[case::no_entry_found("{}", &["a"], false)]
#[case::no_keys(r#"{"a":{}}"#, &[], false)]
#[case::a_scalar_is_not_an_entry(r#"{"a":5}"#, &["a"], false)]
fn cost_map_omits_token_price_cases(
    #[case] live: &str,
    #[case] keys: &[&str],
    #[case] expected: bool,
) {
    assert_eq!(
        live_registry(live).cost_map_omits_token_price(keys),
        expected
    );
}

#[rstest]
#[case::rule_match_outside_the_cost_map("{}", r#"{"key":"ft:x"}"#, true)]
#[case::key_in_the_cost_map(r#"{"ft:x":{}}"#, r#"{"key":"ft:x"}"#, false)]
#[case::no_rule_matches("{}", r#"{"key":"other"}"#, false)]
#[case::no_key("{}", "{}", false)]
#[case::key_is_not_text("{}", r#"{"key":5}"#, false)]
fn register_generalized_info_cases(
    #[case] model_cost: &str,
    #[case] info: &str,
    #[case] expected: bool,
) {
    let world = World::new("{}", FT_RULES);
    assert_eq!(
        is_generalized_model_info(&object(model_cost), &object(info), &world.generalizations),
        expected
    );
}

#[tokio::test]
async fn register_known_model_merges_over_the_built_in_entry() {
    let mut registry = live_registry(
        r#"{"gpt-4o":{"litellm_provider":"openai","mode":"chat","input_cost_per_token":0.0000025}}"#,
    );
    let world = World::new(
        r#"{"gpt-4o":{"key":"gpt-4o","litellm_provider":"openai","mode":"chat","input_cost_per_token":0.0000025,"output_cost_per_token":0}}"#,
        NO_RULES,
    );
    let value = r#"{"gpt-4o":{"litellm_provider":"openai","output_cost_per_token":"3e-07","max_tokens":"8192"}}"#;
    let registered = register_entries(&mut registry, &world, value).await;
    assert_eq!(
        registry.model_cost.get("gpt-4o"),
        Some(&json(
            r#"{"litellm_provider":"openai","mode":"chat","input_cost_per_token":0.0000025,"output_cost_per_token":3e-7,"max_tokens":8192,"key":"gpt-4o"}"#
        ))
    );
    assert_eq!(registered.warnings, warnings(&[]));
}

#[rstest]
#[case::not_in_the_cost_map("{}", false)]
#[case::already_in_the_cost_map(r#"{"ft:gpt-4o":{"mode":"chat"}}"#, true)]
#[tokio::test]
async fn register_uses_the_built_in_info_unless_a_rule_generalized_it(
    #[case] live: &str,
    #[case] info_used: bool,
) {
    let mut registry = live_registry(live);
    let world = World::new(
        r#"{"ft:gpt-4o":{"key":"ft:gpt-4o","mode":"chat","max_tokens":4096,"from_info":true}}"#,
        FT_RULES,
    );
    let value = r#"{"ft:gpt-4o":{"litellm_provider":"openai","input_cost_per_token":0.000001}}"#;
    let registered = register_entries(&mut registry, &world, value).await;
    let from_rules = r#"{"mode":"chat","max_tokens":4096,"litellm_provider":"openai","input_cost_per_token":0.000001}"#;
    let from_info = r#"{"mode":"chat","max_tokens":4096,"key":"ft:gpt-4o","from_info":true,"litellm_provider":"openai","input_cost_per_token":0.000001}"#;
    let expected = if info_used { from_info } else { from_rules };
    assert_eq!(registry.model_cost.get("ft:gpt-4o"), Some(&json(expected)));
    let expected_warnings = if info_used { &[] } else { &["ft:gpt-4o"][..] };
    assert_eq!(registered.warnings, warnings(expected_warnings));
}

#[rstest]
#[case::priced_without_cache_prices(
    r#"{"m":{"litellm_provider":"x","input_cost_per_token":0.1}}"#,
    None,
    &["m"]
)]
#[case::output_price_only(
    r#"{"m":{"litellm_provider":"x","output_cost_per_token":0.1}}"#,
    None,
    &["m"]
)]
#[case::display_name_replaces_the_key(
    r#"{"m":{"litellm_provider":"x","input_cost_per_token":0.1}}"#,
    Some("Deployment"),
    &["Deployment"]
)]
#[case::cache_read_declared(
    r#"{"m":{"input_cost_per_token":0.1,"cache_read_input_token_cost":0.01}}"#,
    None,
    &[]
)]
#[case::cache_creation_declared(
    r#"{"m":{"input_cost_per_token":0.1,"cache_creation_input_token_cost":0.01}}"#,
    None,
    &[]
)]
#[case::tiered_pricing_declared(
    r#"{"m":{"input_cost_per_token":0.1,"tiered_pricing":{"tier":1}}}"#,
    None,
    &[]
)]
#[case::a_null_cache_price_counts_as_absent(
    r#"{"m":{"input_cost_per_token":0.1,"cache_read_input_token_cost":null}}"#,
    None,
    &["m"]
)]
#[case::no_prices(r#"{"m":{"mode":"chat"}}"#, None, &[])]
#[case::null_prices(
    r#"{"m":{"input_cost_per_token":null,"output_cost_per_token":null}}"#,
    None,
    &[]
)]
#[tokio::test]
async fn register_new_model_warns_about_missing_cache_pricing(
    #[case] value: &str,
    #[case] display_name: Option<&str>,
    #[case] expected: &[&str],
) {
    let mut registry = live_registry("{}");
    let world = World::new("{}", NO_RULES);
    let options = RegisterOptions {
        persist_across_reloads: false,
        warning_display_name: display_name,
    };
    let source = ModelCostSource::Entries(entries(value));
    let registered = try_register(&mut registry, &world, source, &options)
        .await
        .unwrap();
    assert_eq!(registered.warnings, warnings(expected));
    assert!(registry.model_cost.contains_key("m"));
}

#[rstest]
#[case::provider_prefix_is_stripped(
    r#"{"gpt-5-mini":{"litellm_provider":"openai","cache_read_input_token_cost":0.0000001,"cache_creation_input_token_cost":null}}"#,
    "openai/gpt-5-mini",
    "openai",
    true,
    false
)]
#[case::bedrock_region_prefix_is_stripped(
    r#"{"claude-x":{"litellm_provider":"bedrock","cache_read_input_token_cost":0.0000001}}"#,
    "us.claude-x",
    "bedrock",
    true,
    false
)]
#[case::repeated_provider_prefixes(
    r#"{"claude-y":{"litellm_provider":"bedrock","cache_read_input_token_cost":0.0000001}}"#,
    "bedrock/bedrock/us.claude-y",
    "bedrock",
    true,
    false
)]
#[case::an_entry_without_a_provider_is_skipped(
    r#"{"us.z":{"litellm_provider":null,"cache_read_input_token_cost":0.5},"z":{"litellm_provider":"openai","cache_read_input_token_cost":0.0000001}}"#,
    "openai/us.z",
    "openai",
    true,
    false
)]
#[case::the_exact_key_is_not_a_candidate(
    r#"{"gpt-5-mini":{"litellm_provider":"openai","cache_read_input_token_cost":0.0000001}}"#,
    "gpt-5-mini",
    "openai",
    false,
    true
)]
#[case::no_provider_on_the_entry(
    r#"{"gpt-5-mini":{"cache_read_input_token_cost":0.0000001}}"#,
    "openai/gpt-5-mini",
    "openai",
    false,
    true
)]
#[case::a_null_cache_price_is_not_inherited(
    r#"{"gpt-5-mini":{"litellm_provider":"openai","cache_read_input_token_cost":null}}"#,
    "openai/gpt-5-mini",
    "openai",
    false,
    false
)]
#[tokio::test]
async fn register_new_model_inherits_cache_pricing_from_a_built_in_variant(
    #[case] live: &str,
    #[case] key: &str,
    #[case] provider: &str,
    #[case] inherits: bool,
    #[case] warns: bool,
) {
    let mut registry = live_registry(live);
    let world = World::new("{}", NO_RULES);
    let value = format!(
        r#"{{"{key}":{{"litellm_provider":"{provider}","input_cost_per_token":0.000001}}}}"#
    );
    let registered = register_entries(&mut registry, &world, &value).await;
    let cache = if inherits {
        r#","cache_read_input_token_cost":0.0000001"#
    } else {
        ""
    };
    let expected =
        format!(r#"{{"litellm_provider":"{provider}","input_cost_per_token":0.000001{cache}}}"#);
    assert_eq!(registry.model_cost.get(key), Some(&json(&expected)));
    let expected_warnings = if warns { &[key][..] } else { &[] };
    assert_eq!(registered.warnings, warnings(expected_warnings));
}

#[rstest]
#[case::provider_github_copilot(
    "gpt-4o",
    "github_copilot",
    "{}",
    true,
    r#"{"litellm_provider":"github_copilot","input_cost_per_token":0.1}"#
)]
#[case::provider_chatgpt(
    "gpt-5",
    "chatgpt",
    r#"{"gpt-5":{"mode":"chat"}}"#,
    true,
    r#"{"mode":"chat","litellm_provider":"chatgpt","input_cost_per_token":0.1}"#
)]
#[case::key_prefix_github_copilot(
    "github_copilot/gpt-4o",
    "openai",
    r#"{"github_copilot/gpt-4o":{"mode":"chat"}}"#,
    true,
    r#"{"mode":"chat","litellm_provider":"openai","input_cost_per_token":0.1}"#
)]
#[case::key_prefix_chatgpt(
    "chatgpt/gpt-5",
    "openai",
    "{}",
    true,
    r#"{"litellm_provider":"openai","input_cost_per_token":0.1}"#
)]
#[case::prefix_needs_its_slash(
    "chatgpt-4o",
    "openai",
    r#"{"chatgpt-4o":{"mode":"chat"}}"#,
    false,
    r#"{"mode":"chat"}"#
)]
#[case::other_providers_consult_the_lookup(
    "gpt-4o",
    "openai",
    r#"{"gpt-4o":{"mode":"chat"}}"#,
    false,
    r#"{"mode":"chat"}"#
)]
#[tokio::test]
async fn register_skips_the_lookup_for_providers_that_authenticate_on_provider_info(
    #[case] key: &str,
    #[case] provider: &str,
    #[case] live: &str,
    #[case] skips_lookup: bool,
    #[case] expected: &str,
) {
    let mut registry = live_registry(live);
    let world = World::new(
        &format!(r#"{{"{key}":{{"key":"other-key","from_info":true}}}}"#),
        NO_RULES,
    );
    let value =
        format!(r#"{{"{key}":{{"litellm_provider":"{provider}","input_cost_per_token":0.1}}}}"#);
    let registered = register_entries(&mut registry, &world, &value).await;
    assert_eq!(registry.model_cost.get(key), Some(&json(expected)));
    assert_eq!(registry.model_cost.contains_key("other-key"), !skips_lookup);
    assert_eq!(registered.warnings, warnings(&[]));
}

#[tokio::test]
async fn register_fetches_a_url_with_one_attempt() {
    let mut registry = live_registry("{}");
    let world = World::new("{}", NO_RULES);
    let url = "https://maps.test/cost.json";
    let source = ModelCostSource::Url(url.to_owned());
    let registered = try_register(&mut registry, &world, source.clone(), &PLAIN)
        .await
        .unwrap();
    assert_eq!(registered.model_cost, source);
    assert_eq!(
        registry.model_cost.get(url),
        Some(&json(r#"{"litellm_provider":"openai","attempts":1}"#))
    );
}

#[tokio::test]
async fn register_reports_a_failed_fetch_and_changes_nothing() {
    let mut registry = live_registry("{}");
    let mut world = World::new("{}", NO_RULES);
    world.fetcher.fails = true;
    let options = RegisterOptions {
        persist_across_reloads: true,
        warning_display_name: None,
    };
    let source = ModelCostSource::Url("https://maps.test/cost.json".to_owned());
    let failure = try_register(&mut registry, &world, source, &options)
        .await
        .unwrap_err();
    assert!(matches!(
        &failure,
        RegistrationError::Fetch { url, .. } if url == "https://maps.test/cost.json"
    ));
    assert!(registry.model_cost.is_empty());
    assert!(registry.runtime_registered.is_empty());
    assert_eq!(world.cleared(), 0);
}

#[tokio::test]
async fn register_returns_the_entries_it_was_given() {
    let mut registry = live_registry("{}");
    let world = World::new("{}", NO_RULES);
    let given = entries(r#"{"m":{"mode":"chat","max_tokens":"8192"}}"#);
    let registered = try_register(
        &mut registry,
        &world,
        ModelCostSource::Entries(given.clone()),
        &PLAIN,
    )
    .await
    .unwrap();
    assert_eq!(registered.model_cost, ModelCostSource::Entries(given));
}

#[rstest]
#[case::persisting(true, r#"{"m":{"mode":"chat"}}"#)]
#[case::not_persisting(false, "{}")]
#[tokio::test]
async fn register_remembers_entries_only_when_asked(
    #[case] persist_across_reloads: bool,
    #[case] remembered: &str,
) {
    let mut registry = live_registry("{}");
    let world = World::new("{}", NO_RULES);
    let options = RegisterOptions {
        persist_across_reloads,
        warning_display_name: None,
    };
    let source = ModelCostSource::Entries(entries(r#"{"m":{"mode":"chat"}}"#));
    try_register(&mut registry, &world, source, &options)
        .await
        .unwrap();
    assert_eq!(registry.runtime_registered, entries(remembered));
}

#[tokio::test]
async fn register_remembers_the_fetched_entries_not_the_url() {
    let mut registry = live_registry("{}");
    let world = World::new("{}", NO_RULES);
    let options = RegisterOptions {
        persist_across_reloads: true,
        warning_display_name: None,
    };
    let source = ModelCostSource::Url("https://maps.test/cost.json".to_owned());
    try_register(&mut registry, &world, source, &options)
        .await
        .unwrap();
    assert_eq!(
        registry.runtime_registered,
        entries(r#"{"https://maps.test/cost.json":{"litellm_provider":"openai","attempts":1}}"#)
    );
}

#[rstest]
#[case::openai("openai", "m", ProviderModelSet::OpenAiChatCompletion, "m")]
#[case::text_completion_openai(
    "text-completion-openai",
    "m",
    ProviderModelSet::OpenAiTextCompletion,
    "m"
)]
#[case::cohere("cohere", "m", ProviderModelSet::Cohere, "m")]
#[case::anthropic("anthropic", "m", ProviderModelSet::Anthropic, "m")]
#[case::openrouter_drops_the_first_segment(
    "openrouter",
    "openrouter/a/b",
    ProviderModelSet::Openrouter,
    "a/b"
)]
#[case::openrouter_without_a_slash("openrouter", "plain", ProviderModelSet::Openrouter, "plain")]
#[case::vercel_ai_gateway("vercel_ai_gateway", "m", ProviderModelSet::VercelAiGateway, "m")]
#[case::edenai("edenai", "m", ProviderModelSet::Edenai, "m")]
#[case::vertex_text("vertex_ai-text-models", "m", ProviderModelSet::VertexText, "m")]
#[case::vertex_code_text(
    "vertex_ai-code-text-models",
    "m",
    ProviderModelSet::VertexCodeText,
    "m"
)]
#[case::vertex_chat("vertex_ai-chat-models", "m", ProviderModelSet::VertexChat, "m")]
#[case::vertex_code_chat(
    "vertex_ai-code-chat-models",
    "m",
    ProviderModelSet::VertexCodeChat,
    "m"
)]
#[case::ai21("ai21", "m", ProviderModelSet::Ai21, "m")]
#[case::nlp_cloud("nlp_cloud", "m", ProviderModelSet::NlpCloud, "m")]
#[case::aleph_alpha("aleph_alpha", "m", ProviderModelSet::AlephAlpha, "m")]
#[case::bedrock("bedrock", "m", ProviderModelSet::Bedrock, "m")]
#[case::novita("novita", "m", ProviderModelSet::Novita, "m")]
#[tokio::test]
async fn register_adds_the_key_to_its_provider_set(
    #[case] provider: &str,
    #[case] key: &str,
    #[case] set: ProviderModelSet,
    #[case] stored: &str,
) {
    let mut registry = live_registry("{}");
    let world = World::new("{}", NO_RULES);
    let value = format!(r#"{{"{key}":{{"litellm_provider":"{provider}"}}}}"#);
    register_entries(&mut registry, &world, &value).await;
    let mut expected = ProviderModelSets::default();
    expected
        .sets
        .entry(set)
        .or_default()
        .insert(stored.to_owned());
    assert_eq!(registry.provider_sets.sets, expected.sets);
}

#[rstest]
#[case::another_provider(r#"{"m":{"litellm_provider":"mistral"}}"#)]
#[case::no_provider(r#"{"m":{"mode":"chat"}}"#)]
#[case::null_provider(r#"{"m":{"litellm_provider":null}}"#)]
#[tokio::test]
async fn register_leaves_other_providers_out_of_the_sets(#[case] value: &str) {
    let mut registry = live_registry("{}");
    let world = World::new("{}", NO_RULES);
    register_entries(&mut registry, &world, value).await;
    assert_eq!(
        registry.provider_sets.sets,
        ProviderModelSets::default().sets
    );
}

#[rstest]
#[case::no_entries("{}", 0)]
#[case::one_entry(r#"{"a":{"mode":"chat"}}"#, 1)]
#[case::two_entries(r#"{"a":{"mode":"chat"},"b":{"mode":"chat"}}"#, 2)]
#[tokio::test]
async fn register_invalidates_the_key_lookups_once_per_entry(
    #[case] value: &str,
    #[case] count: usize,
) {
    let mut registry = live_registry("{}");
    registry.keys.lowercase_index = Some(HashMap::new());
    registry.keys.generation = 7;
    let world = World::new("{}", NO_RULES);
    register_entries(&mut registry, &world, value).await;
    assert_eq!(registry.keys.lowercase_index.is_some(), count == 0);
    assert_eq!(
        registry.keys.generation,
        7_u64.saturating_add(u64::try_from(count).unwrap())
    );
    assert_eq!(world.cleared(), count);
}

#[rstest]
#[case::number(r#"{"ok":{"mode":"chat"},"bad":{"litellm_provider":5}}"#)]
#[case::bool(r#"{"ok":{"mode":"chat"},"bad":{"litellm_provider":true}}"#)]
#[case::list(r#"{"ok":{"mode":"chat"},"bad":{"litellm_provider":["x"]}}"#)]
#[tokio::test]
async fn register_rejects_a_provider_that_is_not_text(#[case] value: &str) {
    let mut registry = live_registry("{}");
    let world = World::new("{}", NO_RULES);
    let source = ModelCostSource::Entries(entries(value));
    let failure = try_register(&mut registry, &world, source, &PLAIN)
        .await
        .unwrap_err();
    assert!(matches!(
        &failure,
        RegistrationError::Entry(EntryError::ProviderNotText { key }) if key == "bad"
    ));
    assert!(registry.model_cost.contains_key("ok"));
    assert!(!registry.model_cost.contains_key("bad"));
}

#[rstest]
#[case::no_key(r#"{"m":{"mode":"chat"}}"#)]
#[case::key_is_not_text(r#"{"m":{"key":5}}"#)]
#[tokio::test]
async fn register_rejects_built_in_info_without_a_text_key(#[case] info: &str) {
    let mut registry = live_registry("{}");
    let world = World::new(info, NO_RULES);
    let source = ModelCostSource::Entries(entries(r#"{"m":{"mode":"chat"}}"#));
    let failure = try_register(&mut registry, &world, source, &PLAIN)
        .await
        .unwrap_err();
    assert!(matches!(
        &failure,
        RegistrationError::Entry(EntryError::BuiltinInfoWithoutKey { model }) if model == "m"
    ));
    assert!(registry.model_cost.is_empty());
}

#[rstest]
#[case::provider_is_null(r#"{"m":{"key":"m","litellm_provider":null}}"#)]
#[case::provider_is_absent(r#"{"m":{"key":"m"}}"#)]
#[tokio::test]
async fn register_keeps_the_live_provider_when_the_seed_has_none(#[case] info: &str) {
    let mut registry = live_registry(r#"{"m":{"litellm_provider":"azure"}}"#);
    let world = World::new(info, NO_RULES);
    register_entries(&mut registry, &world, r#"{"m":{"mode":"chat"}}"#).await;
    assert_eq!(
        registry.model_cost.get("m"),
        Some(&json(
            r#"{"litellm_provider":"azure","key":"m","mode":"chat"}"#
        ))
    );
}

#[rstest]
#[case::number(r#"{"m":5}"#)]
#[case::null(r#"{"m":null}"#)]
#[case::text(r#"{"m":"text"}"#)]
#[case::absent("{}")]
#[tokio::test]
async fn register_replaces_a_live_entry_that_is_not_an_object(#[case] live: &str) {
    let mut registry = live_registry(live);
    let world = World::new(r#"{"m":{"key":"m"}}"#, NO_RULES);
    register_entries(&mut registry, &world, r#"{"m":{"mode":"chat"}}"#).await;
    assert_eq!(
        registry.model_cost.get("m"),
        Some(&json(r#"{"key":"m","mode":"chat"}"#))
    );
}

#[rstest]
#[case::nothing_states_them(
    r#"{"m":{"litellm_provider":"openai"}}"#,
    "m",
    r#"{"m":{"mode":"chat"}}"#,
    r#"{"litellm_provider":"openai","key":"m","mode":"chat"}"#
)]
#[case::the_raw_entry_states_the_input_price(
    r#"{"m":{"litellm_provider":"openai","input_cost_per_token":0}}"#,
    "m",
    r#"{"m":{"mode":"chat"}}"#,
    r#"{"litellm_provider":"openai","input_cost_per_token":0,"key":"m","mode":"chat"}"#
)]
#[case::the_raw_entry_states_the_output_price(
    r#"{"m":{"litellm_provider":"openai","output_cost_per_token":0}}"#,
    "m",
    r#"{"m":{"mode":"chat"}}"#,
    r#"{"litellm_provider":"openai","output_cost_per_token":0,"key":"m","mode":"chat"}"#
)]
#[case::the_value_states_the_output_price(
    r#"{"m":{"litellm_provider":"openai"}}"#,
    "m",
    r#"{"m":{"output_cost_per_token":"0"}}"#,
    r#"{"litellm_provider":"openai","key":"m","output_cost_per_token":0}"#
)]
#[case::a_null_price_in_the_value_keeps_the_synthesized_zero(
    r#"{"m":{"litellm_provider":"openai"}}"#,
    "m",
    r#"{"m":{"input_cost_per_token":null}}"#,
    r#"{"litellm_provider":"openai","key":"m","input_cost_per_token":0}"#
)]
#[case::the_raw_entry_is_found_under_the_incoming_key(
    r#"{"m":{"litellm_provider":"openai","input_cost_per_token":0}}"#,
    "other",
    r#"{"m":{"mode":"chat"}}"#,
    r#"{"litellm_provider":"openai","input_cost_per_token":0,"key":"other","mode":"chat"}"#
)]
#[case::the_entry_under_the_info_key_wins(
    r#"{"other":{"litellm_provider":"openai"},"m":{"input_cost_per_token":0}}"#,
    "other",
    r#"{"m":{"mode":"chat"}}"#,
    r#"{"litellm_provider":"openai","key":"other","mode":"chat"}"#
)]
#[tokio::test]
async fn register_drops_synthesized_costs_nothing_stated(
    #[case] live: &str,
    #[case] info_key: &str,
    #[case] value: &str,
    #[case] expected: &str,
) {
    let mut registry = live_registry(live);
    let info = format!(
        r#"{{"m":{{"key":"{info_key}","litellm_provider":"openai","input_cost_per_token":0,"output_cost_per_token":0}}}}"#
    );
    let world = World::new(&info, NO_RULES);
    register_entries(&mut registry, &world, value).await;
    assert_eq!(registry.model_cost.get(info_key), Some(&json(expected)));
}

#[tokio::test]
async fn replay_asks_the_live_deployments_before_the_runtime_entries() {
    let mut registry = live_registry("{}");
    registry.runtime_registered = entries(r#"{"k":{"mode":"runtime"}}"#);
    registry.live_replay = Some(Box::new(live_registration(
        r#"{"k":{"mode":"live"}}"#,
        false,
        None,
    )));
    let world = World::new("{}", NO_RULES);
    let replayed = registry
        .reapply_runtime_registrations(&world.environment())
        .await
        .unwrap();
    assert_eq!(replayed, warnings(&[]));
    assert_eq!(
        registry.model_cost.get("k"),
        Some(&json(r#"{"mode":"runtime"}"#))
    );
    assert_eq!(world.cleared(), 2);
}

#[rstest]
#[case::persisting(true, 2)]
#[case::not_persisting(false, 1)]
#[tokio::test]
async fn replay_keeps_a_live_registration_only_when_it_persists(
    #[case] persist_across_reloads: bool,
    #[case] registrations: usize,
) {
    let mut registry = live_registry("{}");
    registry.live_replay = Some(Box::new(live_registration(
        r#"{"k":{"mode":"live"}}"#,
        persist_across_reloads,
        None,
    )));
    let world = World::new("{}", NO_RULES);
    registry
        .reapply_runtime_registrations(&world.environment())
        .await
        .unwrap();
    assert_eq!(
        registry.runtime_registered.contains_key("k"),
        persist_across_reloads
    );
    assert_eq!(world.cleared(), registrations);
}

#[rstest]
#[case::not_persisting(false, &["Deployment A"])]
#[case::persisting(true, &["Deployment A", "k"])]
#[tokio::test]
async fn replay_names_the_deployment_in_its_warning(
    #[case] persist_across_reloads: bool,
    #[case] expected: &[&str],
) {
    let mut registry = live_registry("{}");
    registry.live_replay = Some(Box::new(live_registration(
        r#"{"k":{"litellm_provider":"x","input_cost_per_token":0.1}}"#,
        persist_across_reloads,
        Some("Deployment A"),
    )));
    let world = World::new("{}", NO_RULES);
    let replayed = registry
        .reapply_runtime_registrations(&world.environment())
        .await
        .unwrap();
    assert_eq!(replayed, warnings(expected));
}

#[tokio::test]
async fn replay_restores_the_runtime_entries_after_a_reload() {
    let mut registry = live_registry("{}");
    registry.runtime_registered = entries(r#"{"a":{"litellm_provider":"openai","mode":"chat"}}"#);
    let world = World::new("{}", NO_RULES);
    registry
        .reapply_runtime_registrations(&world.environment())
        .await
        .unwrap();
    assert_eq!(
        registry.model_cost.get("a"),
        Some(&json(r#"{"litellm_provider":"openai","mode":"chat"}"#))
    );
    assert_eq!(
        registry.runtime_registered,
        entries(r#"{"a":{"litellm_provider":"openai","mode":"chat"}}"#)
    );
}

#[tokio::test]
async fn replay_with_nothing_registered_does_nothing() {
    let mut registry = live_registry("{}");
    let world = World::new("{}", NO_RULES);
    let replayed = registry
        .reapply_runtime_registrations(&world.environment())
        .await
        .unwrap();
    assert_eq!(replayed, warnings(&[]));
    assert!(registry.model_cost.is_empty());
    assert_eq!(world.cleared(), 0);
}

#[tokio::test]
async fn replay_stops_at_an_entry_that_cannot_register() {
    let mut registry = live_registry("{}");
    registry.runtime_registered = entries(r#"{"bad":{"litellm_provider":5}}"#);
    let world = World::new("{}", NO_RULES);
    let failure = registry
        .reapply_runtime_registrations(&world.environment())
        .await
        .unwrap_err();
    assert!(matches!(
        failure,
        RegistrationError::Entry(EntryError::ProviderNotText { .. })
    ));
}
