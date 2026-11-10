#![cfg(test)]

use std::collections::BTreeMap;
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::identity::provider_id::ProviderId;
use crate::provider_metadata_factory::selection::ProviderModelMetadata;

use super::cache::AvailableModelsCache;
use super::environment::EnvironmentSnapshot;
use super::listing::{ListingCredentials, ModelListing};
use super::parameters::LiteLlmParams;
use super::{AZURE_MODEL, EnumerationRequest, EnumerationWarning, get_valid_models};

type Catalog = BTreeMap<ProviderId, Vec<String>>;

fn clock_at(now: f64) -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(move || now)
}

fn cache() -> AvailableModelsCache {
    AvailableModelsCache::with_clock(clock_at(0.0))
}

#[derive(Default)]
struct ClockState {
    now: f64,
    fails: bool,
}

fn switchable_clock() -> (Arc<Mutex<ClockState>>, Arc<dyn Fn() -> f64 + Send + Sync>) {
    let state = Arc::new(Mutex::new(ClockState::default()));
    let clock_state = Arc::clone(&state);
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(move || {
        let locked = clock_state.lock().unwrap_or_else(PoisonError::into_inner);
        if locked.fails { f64::NAN } else { locked.now }
    });
    (state, clock)
}

fn snapshot(entries: &[(&str, &str)]) -> EnvironmentSnapshot {
    let map: BTreeMap<String, String> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    EnvironmentSnapshot::from(&map)
}

fn models(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

fn catalog(entries: &[(ProviderId, &[&str])]) -> Catalog {
    entries
        .iter()
        .map(|(provider, names)| (*provider, models(names)))
        .collect()
}

fn params(model: &str, api_key: Option<&str>, api_base: Option<&str>) -> LiteLlmParams {
    LiteLlmParams {
        model: model.to_owned(),
        api_key: api_key.map(SecretString::from),
        api_base: api_base.map(ToOwned::to_owned),
        api_version: None,
        custom_llm_provider: None,
    }
}

fn request<'fixture>(
    provider_list: &'fixture [ProviderId],
    models_by_provider: &'fixture Catalog,
    environment: &'fixture EnvironmentSnapshot,
) -> EnumerationRequest<'fixture> {
    EnumerationRequest {
        check_provider_endpoint: false,
        global_check_provider_endpoint: false,
        custom_llm_provider: None,
        parameters: LiteLlmParams::default(),
        api_key: None,
        api_base: None,
        provider_list,
        models_by_provider,
        environment,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted provider listing failed")]
struct ListingFailed;

#[derive(Debug)]
struct ListingCall {
    class: &'static str,
    api_key: Option<String>,
    api_base: Option<String>,
}

#[derive(Default)]
struct ListingScript {
    outcomes: BTreeMap<&'static str, Result<Vec<String>, ()>>,
    calls: Mutex<Vec<ListingCall>>,
}

impl ListingScript {
    fn answer(&mut self, class: &'static str, listed: Vec<String>) {
        self.outcomes.insert(class, Ok(listed));
    }

    fn fail(&mut self, class: &'static str) {
        self.outcomes.insert(class, Err(()));
    }

    fn take_calls(&self) -> Vec<ListingCall> {
        take(&mut *self.calls.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl ModelListing for ListingScript {
    type Error = ListingFailed;

    fn get_models(
        &self,
        adapter: &ProviderModelMetadata,
        credentials: ListingCredentials<'_>,
    ) -> Result<Vec<String>, ListingFailed> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ListingCall {
                class: adapter.class_name(),
                api_key: credentials
                    .api_key
                    .map(|key| key.expose_secret().to_owned()),
                api_base: credentials.api_base.map(ToOwned::to_owned),
            });
        self.outcomes
            .get(adapter.class_name())
            .cloned()
            .unwrap_or(Err(()))
            .map_err(|()| ListingFailed)
    }
}

#[rstest]
#[case::argument_false_global_false(false, false, false)]
#[case::argument_true_global_false(true, false, true)]
#[case::argument_false_global_true(false, true, true)]
#[case::argument_true_global_true(true, true, true)]
fn endpoint_argument_or_global(
    #[case] argument: bool,
    #[case] global: bool,
    #[case] checked: bool,
) {
    let environment = snapshot(&[("OPENAI_API_KEY", "sk-test")]);
    let provider_list = [ProviderId::Openai];
    let catalog = catalog(&[(ProviderId::Openai, &["static-model"])]);
    let mut listing = ListingScript::default();
    listing.answer("OpenAIGPTConfig", models(&["listed-model"]));
    let mut call = request(&provider_list, &catalog, &environment);
    call.check_provider_endpoint = argument;
    call.global_check_provider_endpoint = global;
    let outcome = get_valid_models(&call, &mut cache(), &listing);
    let expected = if checked {
        models(&["listed-model"])
    } else {
        models(&["static-model"])
    };
    assert_eq!(outcome.models, expected);
    assert_eq!(listing.take_calls().len(), usize::from(checked));
}

#[test]
fn explicit_false_does_not_disable_global_true() {
    let environment = snapshot(&[]);
    let provider_list = [ProviderId::Openai];
    let catalog = catalog(&[]);
    let mut listing = ListingScript::default();
    listing.answer("OpenAIGPTConfig", models(&["listed-model"]));
    let mut call = request(&provider_list, &catalog, &environment);
    call.custom_llm_provider = Some("openai");
    call.check_provider_endpoint = false;
    call.global_check_provider_endpoint = true;
    let outcome = get_valid_models(&call, &mut cache(), &listing);
    assert_eq!(outcome.models, models(&["listed-model"]));
    assert_eq!(listing.take_calls().len(), 1);
}

#[test]
fn parameter_creation_and_explicit_mutation() {
    let environment = snapshot(&[]);
    let provider_list = [ProviderId::Openai];
    let catalog = catalog(&[]);
    let mut listing = ListingScript::default();
    listing.answer("OpenAIGPTConfig", models(&["listed"]));
    let mut cache = cache();
    let mut absent = request(&provider_list, &catalog, &environment);
    absent.custom_llm_provider = Some("openai");
    absent.global_check_provider_endpoint = true;
    absent.api_key = Some(SecretString::from("k1"));
    assert_eq!(
        get_valid_models(&absent, &mut cache, &listing).models,
        models(&["listed"])
    );
    let mut explicit = request(&provider_list, &catalog, &environment);
    explicit.custom_llm_provider = Some("openai");
    explicit.global_check_provider_endpoint = true;
    explicit.parameters = params("", Some("k1"), None);
    assert_eq!(
        get_valid_models(&explicit, &mut cache, &listing).models,
        models(&["listed"])
    );
    assert_eq!(listing.take_calls().len(), 1);
    let mut empty = request(&provider_list, &catalog, &environment);
    empty.custom_llm_provider = Some("openai");
    empty.global_check_provider_endpoint = true;
    empty.parameters = params("", Some("supplied"), Some("https://supplied"));
    empty.api_key = Some(SecretString::from(""));
    empty.api_base = Some(String::new());
    assert_eq!(
        get_valid_models(&empty, &mut cache, &listing).models,
        models(&["listed"])
    );
    let calls = listing.take_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].api_key.as_deref(), Some(""));
    assert_eq!(calls[0].api_base.as_deref(), Some(""));
}

#[test]
fn supplied_parameters_change_the_cache_key() {
    let environment = snapshot(&[]);
    let provider_list = [ProviderId::Openai];
    let catalog = catalog(&[]);
    let mut listing = ListingScript::default();
    listing.answer("OpenAIGPTConfig", models(&["listed"]));
    let mut cache = cache();
    let mut absent = request(&provider_list, &catalog, &environment);
    absent.custom_llm_provider = Some("openai");
    absent.global_check_provider_endpoint = true;
    drop(get_valid_models(&absent, &mut cache, &listing));
    let mut supplied = request(&provider_list, &catalog, &environment);
    supplied.custom_llm_provider = Some("openai");
    supplied.global_check_provider_endpoint = true;
    supplied.parameters = params("supplied-model", None, None);
    drop(get_valid_models(&supplied, &mut cache, &listing));
    assert_eq!(listing.take_calls().len(), 2);
}

#[test]
fn provider_key_names_presence_and_order() {
    let environment = snapshot(&[
        ("OPENAI_API_KEY", ""),
        ("ANTHROPIC_API_KEY", "sk-anthropic"),
        ("TOGETHERAI_API_KEY", "sk-together"),
        ("JINA_AI_API_KEY", "sk-jina"),
        ("AZURE_API_KEY", "sk-azure"),
    ]);
    let provider_list = [
        ProviderId::Openai,
        ProviderId::Anthropic,
        ProviderId::TogetherAi,
        ProviderId::JinaAi,
        ProviderId::Azure,
        ProviderId::Cohere,
        ProviderId::Openai,
    ];
    let catalog = catalog(&[
        (ProviderId::Openai, &["o1", "o2"]),
        (ProviderId::Anthropic, &["a1"]),
        (ProviderId::TogetherAi, &["t1"]),
        (ProviderId::JinaAi, &["j1"]),
        (ProviderId::Azure, &["az-catalog"]),
        (ProviderId::Cohere, &["c1"]),
    ]);
    let outcome = get_valid_models(
        &request(&provider_list, &catalog, &environment),
        &mut cache(),
        &ListingScript::default(),
    );
    assert_eq!(
        outcome.models,
        models(&["o1", "o2", "a1", "t1", "j1", AZURE_MODEL, "o1", "o2"])
    );
}

#[test]
fn provider_key_presence_including_empty() {
    let empty = snapshot(&[("OPENAI_API_KEY", "")]);
    let absent = snapshot(&[("COHERE_API_KEY", "sk-cohere")]);
    let provider_list = [ProviderId::Openai, ProviderId::Cohere];
    let catalog = catalog(&[(ProviderId::Openai, &["o1"]), (ProviderId::Cohere, &["c1"])]);
    let with_empty = get_valid_models(
        &request(&provider_list, &catalog, &empty),
        &mut cache(),
        &ListingScript::default(),
    );
    assert_eq!(with_empty.models, models(&["o1"]));
    let without_key = get_valid_models(
        &request(&provider_list, &catalog, &absent),
        &mut cache(),
        &ListingScript::default(),
    );
    assert_eq!(without_key.models, models(&["c1"]));
}

#[test]
fn registry_only_provider_takes_the_static_path() {
    let environment = snapshot(&[("GMI_API_KEY", "sk")]);
    let provider_list = [ProviderId::Gmi];
    let catalog = catalog(&[(ProviderId::Gmi, &["gmi-a"])]);
    let mut call = request(&provider_list, &catalog, &environment);
    call.custom_llm_provider = Some("gmi");
    let outcome = get_valid_models(&call, &mut cache(), &ListingScript::default());
    assert_eq!(outcome.models, models(&["gmi-a"]));
    assert!(outcome.warnings.is_empty());
}

#[test]
fn custom_provider_singleton() {
    let environment = snapshot(&[("OPENAI_API_KEY", "sk"), ("ANTHROPIC_API_KEY", "sk-a")]);
    let provider_list = [ProviderId::Anthropic, ProviderId::Openai];
    let catalog = catalog(&[
        (ProviderId::Anthropic, &["a1"]),
        (ProviderId::Openai, &["o1"]),
    ]);
    let mut call = request(&provider_list, &catalog, &environment);
    call.custom_llm_provider = Some("openai");
    let outcome = get_valid_models(&call, &mut cache(), &ListingScript::default());
    assert_eq!(outcome.models, models(&["o1"]));
}

#[test]
fn azure_literal() {
    let environment = snapshot(&[("AZURE_API_KEY", "sk")]);
    let provider_list = [ProviderId::Azure, ProviderId::Azure];
    let catalog = catalog(&[(ProviderId::Azure, &["az-catalog"])]);
    let mut call = request(&provider_list, &catalog, &environment);
    call.check_provider_endpoint = true;
    call.global_check_provider_endpoint = true;
    let listing = ListingScript::default();
    let outcome = get_valid_models(&call, &mut cache(), &listing);
    assert_eq!(outcome.models, models(&[AZURE_MODEL, AZURE_MODEL]));
    assert!(listing.take_calls().is_empty());
}

#[test]
fn static_catalog_copy_and_duplicates() {
    let environment = snapshot(&[("OPENAI_API_KEY", "sk")]);
    let provider_list = [ProviderId::Openai, ProviderId::Cohere];
    let catalog = catalog(&[(ProviderId::Openai, &["a", "a", "b"])]);
    let outcome = get_valid_models(
        &request(&provider_list, &catalog, &environment),
        &mut cache(),
        &ListingScript::default(),
    );
    assert_eq!(outcome.models, models(&["a", "a", "b"]));
    assert_eq!(catalog[&ProviderId::Openai], models(&["a", "a", "b"]));
}

#[rstest]
#[case::unknown_name("not-a-provider")]
#[case::empty_name("")]
fn unsupported_provider_outer_warn_drops_all(#[case] name: &str) {
    let environment = snapshot(&[("OPENAI_API_KEY", "sk")]);
    let provider_list = [ProviderId::Openai];
    let catalog = catalog(&[(ProviderId::Openai, &["o1"])]);
    let mut call = request(&provider_list, &catalog, &environment);
    call.custom_llm_provider = Some(name);
    let outcome = get_valid_models(&call, &mut cache(), &ListingScript::default());
    assert_eq!(outcome.models, Vec::<String>::new());
    assert_eq!(
        outcome.warnings,
        vec![EnumerationWarning::ErrorGettingValidModels]
    );
}

#[test]
fn endpoint_error_warn_keeps_later_providers() {
    let environment = snapshot(&[
        ("OPENAI_API_KEY", "sk"),
        ("ANTHROPIC_API_KEY", "sk-a"),
        ("COHERE_API_KEY", "sk-c"),
    ]);
    let provider_list = [
        ProviderId::Openai,
        ProviderId::Anthropic,
        ProviderId::Cohere,
    ];
    let catalog = catalog(&[(ProviderId::Cohere, &["c1"])]);
    let mut listing = ListingScript::default();
    listing.fail("OpenAIGPTConfig");
    listing.answer("AnthropicModelInfo", models(&["a-listed"]));
    let mut call = request(&provider_list, &catalog, &environment);
    call.global_check_provider_endpoint = true;
    let outcome = get_valid_models(&call, &mut cache(), &listing);
    assert_eq!(outcome.models, models(&["a-listed", "c1"]));
    assert_eq!(
        outcome.warnings,
        vec![EnumerationWarning::ErrorGettingValidModels]
    );
    assert_eq!(listing.take_calls().len(), 2);
}

#[test]
fn empty_cached_list_hit() {
    let environment = snapshot(&[]);
    let provider_list = [ProviderId::Openai];
    let catalog = catalog(&[]);
    let mut listing = ListingScript::default();
    listing.answer("OpenAIGPTConfig", Vec::new());
    let mut call = request(&provider_list, &catalog, &environment);
    call.custom_llm_provider = Some("openai");
    call.global_check_provider_endpoint = true;
    let mut cache = cache();
    let first = get_valid_models(&call, &mut cache, &listing);
    let second = get_valid_models(&call, &mut cache, &listing);
    assert_eq!(first.models, Vec::<String>::new());
    assert_eq!(second.models, Vec::<String>::new());
    assert_eq!(listing.take_calls().len(), 1);
}

#[rstest]
#[case::supplied_params(None, None, Some("pk"), Some("https://p"))]
#[case::explicit_overrides(Some("pk"), Some("https://p"), Some("ek"), Some("https://e"))]
fn get_models_only_receives_key_and_base(
    #[case] supplied_key: Option<&str>,
    #[case] supplied_base: Option<&str>,
    #[case] request_key: Option<&str>,
    #[case] request_base: Option<&str>,
) {
    let environment = snapshot(&[]);
    let provider_list = [ProviderId::Openai];
    let catalog = catalog(&[]);
    let mut listing = ListingScript::default();
    listing.answer("OpenAIGPTConfig", models(&["listed"]));
    let mut call = request(&provider_list, &catalog, &environment);
    call.custom_llm_provider = Some("openai");
    call.global_check_provider_endpoint = true;
    call.parameters = params("m", supplied_key, supplied_base);
    call.api_key = request_key.map(SecretString::from);
    call.api_base = request_base.map(ToOwned::to_owned);
    drop(get_valid_models(&call, &mut cache(), &listing));
    let calls = listing.take_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].class, "OpenAIGPTConfig");
    assert_eq!(calls[0].api_key.as_deref(), request_key.or(supplied_key));
    assert_eq!(calls[0].api_base.as_deref(), request_base.or(supplied_base));
}

#[test]
fn cache_capacity300_ttl1000() {
    let cache = cache();
    assert_eq!(cache.limits.capacity, 300_usize);
    assert_eq!(cache.limits.default_ttl.to_bits(), 1000.0_f64.to_bits());
}

#[test]
fn cache_copy_on_read_write() {
    let environment = snapshot(&[]);
    let parameters = params("m", None, None);
    let mut cache = cache();
    let mut written = models(&["a"]);
    cache
        .set(ProviderId::Openai, Some(&parameters), &written)
        .unwrap();
    written.push("b".to_owned());
    let mut first = cache
        .get(Some(ProviderId::Openai), Some(&parameters), &environment)
        .unwrap()
        .unwrap();
    assert_eq!(first, models(&["a"]));
    first.push("c".to_owned());
    let second = cache
        .get(Some(ProviderId::Openai), Some(&parameters), &environment)
        .unwrap()
        .unwrap();
    assert_eq!(second, models(&["a"]));
}

#[rstest]
#[case::different_model("other", Some("key-one"), None, None)]
#[case::different_key("m", Some("key-two"), None, None)]
#[case::different_base("m", Some("key-one"), Some("https://base"), None)]
#[case::different_api_version("m", Some("key-one"), None, Some("2024-01-01"))]
fn full_params_provider_cache_isolation(
    #[case] model: &str,
    #[case] key: Option<&str>,
    #[case] base: Option<&str>,
    #[case] api_version: Option<&str>,
) {
    let environment = snapshot(&[]);
    let stored = params("m", Some("key-one"), None);
    let variant = LiteLlmParams {
        model: model.to_owned(),
        api_key: key.map(SecretString::from),
        api_base: base.map(ToOwned::to_owned),
        api_version: api_version.map(ToOwned::to_owned),
        custom_llm_provider: None,
    };
    let mut cache = cache();
    cache
        .set(ProviderId::Openai, Some(&stored), &models(&["stored"]))
        .unwrap();
    assert!(
        cache
            .get(Some(ProviderId::Openai), Some(&variant), &environment)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), Some(&stored), &environment)
            .unwrap(),
        Some(models(&["stored"]))
    );
    assert!(
        cache
            .get(Some(ProviderId::Anthropic), Some(&stored), &environment)
            .unwrap()
            .is_none()
    );
}

#[test]
fn custom_provider_params_isolate_the_cache() {
    let environment = snapshot(&[]);
    let stored = params("m", Some("key-one"), None);
    let mut variant = stored.clone();
    variant.custom_llm_provider = Some(ProviderId::Anthropic);
    let mut cache = cache();
    cache
        .set(ProviderId::Openai, Some(&stored), &models(&["stored"]))
        .unwrap();
    assert!(
        cache
            .get(Some(ProviderId::Openai), Some(&variant), &environment)
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[case::openai("OPENAI_API_KEY", true)]
#[case::anthropic("ANTHROPIC_API_KEY", true)]
#[case::azure("AZURE_REGION", true)]
#[case::aws("AWS_DEFAULT_REGION", true)]
#[case::prefix_alone("OPENAI", true)]
#[case::lowercase("openai_api_key", false)]
#[case::path("PATH", false)]
#[case::embedded_prefix("XOPENAI_API_KEY", false)]
fn fingerprint_prefixes_and_first_clear(#[case] variable: &str, #[case] changes: bool) {
    let environment = snapshot(&[]);
    let mut cache = cache();
    cache
        .set(ProviderId::Openai, None, &models(&["stale"]))
        .unwrap();
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &environment)
            .unwrap(),
        None
    );
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &environment)
            .unwrap(),
        None
    );
    cache
        .set(ProviderId::Openai, None, &models(&["fresh"]))
        .unwrap();
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &environment)
            .unwrap(),
        Some(models(&["fresh"]))
    );
    let changed = snapshot(&[(variable, "value")]);
    let result = cache.get(Some(ProviderId::Openai), None, &changed).unwrap();
    let expected = if changes {
        None
    } else {
        Some(models(&["fresh"]))
    };
    assert_eq!(result, expected);
}

#[test]
fn fingerprint_covers_values() {
    let first = snapshot(&[("OPENAI_API_KEY", "one")]);
    let second = snapshot(&[("OPENAI_API_KEY", "two")]);
    let mut cache = cache();
    assert_eq!(
        cache.get(Some(ProviderId::Openai), None, &first).unwrap(),
        None
    );
    cache
        .set(ProviderId::Openai, None, &models(&["m"]))
        .unwrap();
    assert_eq!(
        cache.get(Some(ProviderId::Openai), None, &first).unwrap(),
        Some(models(&["m"]))
    );
    assert_eq!(
        cache.get(Some(ProviderId::Openai), None, &second).unwrap(),
        None
    );
}

#[test]
fn fingerprint_never_updated_after_change() {
    let original = snapshot(&[("OPENAI_API_KEY", "one")]);
    let changed = snapshot(&[("OPENAI_API_KEY", "two")]);
    let other = snapshot(&[("ANTHROPIC_API_KEY", "three")]);
    let mut cache = cache();
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &original)
            .unwrap(),
        None
    );
    cache
        .set(ProviderId::Openai, None, &models(&["one"]))
        .unwrap();
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &original)
            .unwrap(),
        Some(models(&["one"]))
    );
    assert_eq!(
        cache.get(Some(ProviderId::Openai), None, &changed).unwrap(),
        None
    );
    cache
        .set(ProviderId::Openai, None, &models(&["two"]))
        .unwrap();
    assert_eq!(
        cache.get(Some(ProviderId::Openai), None, &other).unwrap(),
        None
    );
    cache
        .set(ProviderId::Openai, None, &models(&["three"]))
        .unwrap();
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &original)
            .unwrap(),
        Some(models(&["three"]))
    );
}

#[test]
fn supplied_params_bypass_env_check() {
    let parameters = params("m", None, None);
    let first = snapshot(&[("OPENAI_API_KEY", "one")]);
    let second = snapshot(&[("OPENAI_API_KEY", "two")]);
    let mut cache = cache();
    cache
        .set(ProviderId::Openai, Some(&parameters), &models(&["m"]))
        .unwrap();
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), Some(&parameters), &first)
            .unwrap(),
        Some(models(&["m"]))
    );
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), Some(&parameters), &second)
            .unwrap(),
        Some(models(&["m"]))
    );
}

#[test]
fn cache_failure_warn_drop() {
    let environment = snapshot(&[("OPENAI_API_KEY", "sk"), ("COHERE_API_KEY", "sk-c")]);
    let provider_list = [ProviderId::Openai, ProviderId::Cohere];
    let catalog = catalog(&[(ProviderId::Cohere, &["c1"])]);
    let mut listing = ListingScript::default();
    listing.answer("OpenAIGPTConfig", models(&["listed"]));
    let mut call = request(&provider_list, &catalog, &environment);
    call.global_check_provider_endpoint = true;
    let (state, clock) = switchable_clock();
    state.lock().unwrap().fails = true;
    let mut failing = AvailableModelsCache::with_clock(clock);
    let outcome = get_valid_models(&call, &mut failing, &listing);
    assert_eq!(outcome.models, models(&["c1"]));
    assert_eq!(
        outcome.warnings,
        vec![EnumerationWarning::ErrorGettingValidModels]
    );
    assert_eq!(listing.take_calls().len(), 1);
}

#[test]
fn cache_read_failure_warn_drop() {
    let environment = snapshot(&[]);
    let provider_list = [ProviderId::Openai];
    let catalog = catalog(&[]);
    let mut listing = ListingScript::default();
    listing.answer("OpenAIGPTConfig", models(&["listed"]));
    let mut call = request(&provider_list, &catalog, &environment);
    call.custom_llm_provider = Some("openai");
    call.global_check_provider_endpoint = true;
    let (state, clock) = switchable_clock();
    let mut cache = AvailableModelsCache::with_clock(clock);
    cache
        .set(
            ProviderId::Openai,
            Some(&LiteLlmParams::default()),
            &models(&["stale"]),
        )
        .unwrap();
    state.lock().unwrap().fails = true;
    let outcome = get_valid_models(&call, &mut cache, &listing);
    assert_eq!(outcome.models, Vec::<String>::new());
    assert_eq!(
        outcome.warnings,
        vec![EnumerationWarning::ErrorGettingValidModels]
    );
    assert!(listing.take_calls().is_empty());
}

#[test]
fn cleared_values_keep_expiries() {
    let environment = snapshot(&[]);
    let now = Arc::new(Mutex::new(0.0_f64));
    let clock_now = Arc::clone(&now);
    let clock = Arc::new(move || *clock_now.lock().unwrap_or_else(PoisonError::into_inner));
    let mut cache = AvailableModelsCache::with_clock(clock);
    cache
        .set(ProviderId::Openai, None, &models(&["m"]))
        .unwrap();
    *now.lock().unwrap_or_else(PoisonError::into_inner) = 500.0_f64;
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &environment)
            .unwrap(),
        None
    );
    cache
        .set(ProviderId::Openai, None, &models(&["n"]))
        .unwrap();
    *now.lock().unwrap_or_else(PoisonError::into_inner) = 1200.0_f64;
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &environment)
            .unwrap(),
        None
    );
}

#[test]
fn cache_reads_until_the_expiry_is_past() {
    let environment = snapshot(&[]);
    let now = Arc::new(Mutex::new(0.0_f64));
    let clock_now = Arc::clone(&now);
    let clock = Arc::new(move || *clock_now.lock().unwrap_or_else(PoisonError::into_inner));
    let mut cache = AvailableModelsCache::with_clock(clock);
    cache
        .set(ProviderId::Openai, None, &models(&["m"]))
        .unwrap();
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &environment)
            .unwrap(),
        None
    );
    cache
        .set(ProviderId::Openai, None, &models(&["m"]))
        .unwrap();
    *now.lock().unwrap_or_else(PoisonError::into_inner) = 1000.0_f64;
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &environment)
            .unwrap(),
        Some(models(&["m"]))
    );
    *now.lock().unwrap_or_else(PoisonError::into_inner) = 1000.5_f64;
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &environment)
            .unwrap(),
        None
    );
    assert!(cache.ttl_entries().is_empty());
}

#[test]
fn fresh_expiries_start_at_the_read_clock() {
    let environment = snapshot(&[]);
    let (state, clock) = switchable_clock();
    let parameters = params("m", None, None);
    let mut cache = AvailableModelsCache::with_clock(clock);
    state.lock().unwrap().now = 500.0_f64;
    cache
        .set(ProviderId::Openai, Some(&parameters), &models(&["v"]))
        .unwrap();
    state.lock().unwrap().now = 1400.0_f64;
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), Some(&parameters), &environment)
            .unwrap(),
        Some(models(&["v"]))
    );
    state.lock().unwrap().now = 1501.0_f64;
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), Some(&parameters), &environment)
            .unwrap(),
        None
    );
}

#[test]
fn cache_evicts_the_earliest_expiry_at_capacity() {
    let environment = snapshot(&[]);
    let (state, clock) = switchable_clock();
    let mut cache = AvailableModelsCache::with_clock(clock);
    (0..299_usize).for_each(|index| {
        let parameters = params(&format!("model-{index}"), None, None);
        cache
            .set(ProviderId::Openai, Some(&parameters), &models(&["value"]))
            .unwrap();
    });
    state.lock().unwrap().now = 500.0_f64;
    let fresh = params("model-fresh", None, None);
    cache
        .set(ProviderId::Openai, Some(&fresh), &models(&["fresh"]))
        .unwrap();
    assert_eq!(cache.ttl_entries().len(), 300_usize);
    let extra = params("model-extra", None, None);
    cache
        .set(ProviderId::Openai, Some(&extra), &models(&["extra"]))
        .unwrap();
    assert_eq!(cache.ttl_entries().len(), 300_usize);
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), Some(&fresh), &environment)
            .unwrap(),
        Some(models(&["fresh"]))
    );
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), Some(&extra), &environment)
            .unwrap(),
        Some(models(&["extra"]))
    );
}

#[test]
fn capacity_after_environment_clear_evicts_live_values() {
    let environment = snapshot(&[]);
    let (state, clock) = switchable_clock();
    let mut cache = AvailableModelsCache::with_clock(clock);
    (0..300_usize).for_each(|index| {
        let parameters = params(&format!("old-{index}"), None, None);
        cache
            .set(ProviderId::Openai, Some(&parameters), &models(&["old"]))
            .unwrap();
    });
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), None, &environment)
            .unwrap(),
        None
    );
    (0..301_usize).for_each(|index| {
        state.lock().unwrap().now =
            f64::from(u32::try_from(index).unwrap()).mul_add(1.0_f64, 500.0_f64);
        let parameters = params(&format!("new-{index}"), None, None);
        cache
            .set(ProviderId::Openai, Some(&parameters), &models(&["new"]))
            .unwrap();
    });
    let evicted = params("new-0", None, None);
    assert_eq!(
        cache
            .get(Some(ProviderId::Openai), Some(&evicted), &environment)
            .unwrap(),
        None
    );
    let present = (0..301_usize)
        .filter(|index| {
            let parameters = params(&format!("new-{index}"), None, None);
            cache
                .get(Some(ProviderId::Openai), Some(&parameters), &environment)
                .unwrap()
                .is_some()
        })
        .count();
    assert_eq!(present, 300_usize);
}

#[test]
fn expiry_pruning_bounds_the_expiry_map() {
    let (state, clock) = switchable_clock();
    let mut cache = AvailableModelsCache::with_clock(clock);
    (0..3_usize).for_each(|index| {
        let parameters = params(&format!("model-{index}"), None, None);
        cache
            .set(ProviderId::Openai, Some(&parameters), &models(&["v"]))
            .unwrap();
    });
    assert_eq!(cache.ttl_entries().len(), 3_usize);
    state.lock().unwrap().now = 1000.0_f64;
    let fresh = params("model-fresh", None, None);
    cache
        .set(ProviderId::Openai, Some(&fresh), &models(&["v"]))
        .unwrap();
    assert_eq!(cache.ttl_entries().len(), 1_usize);
    (3..6_usize).for_each(|index| {
        let parameters = params(&format!("model-{index}"), None, None);
        cache
            .set(ProviderId::Openai, Some(&parameters), &models(&["v"]))
            .unwrap();
    });
    assert_eq!(cache.ttl_entries().len(), 4_usize);
    state.lock().unwrap().now = 5000.0_f64;
    let later = params("model-later", None, None);
    cache
        .set(ProviderId::Openai, Some(&later), &models(&["v"]))
        .unwrap();
    assert_eq!(cache.ttl_entries().len(), 1_usize);
}
