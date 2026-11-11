use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use secrecy::SecretString;

use crate::decisions::Json;
use crate::full_model_metadata::cache::{CacheInfo, CacheKey, ModelInfoCache};
use crate::full_model_metadata::error::{FullMetadataError, ModelRecordError};
use crate::full_model_metadata::parameters::ProviderParameterSource;
use crate::full_model_metadata::record::ModelInfo;
use crate::identity::provider_id::ProviderId;
use crate::model_metadata_resolution::error::ModelMetadataError;
use crate::params::CallType;
use crate::params::capabilities::{
    CapabilityQuery, ModelProjection, OpenAiCapability, OpenAiConfigTag, ParameterCapabilities,
    supported_parameters,
};
use crate::provider_metadata_factory::adapters::DynamicModelInfo;

use super::{
    Fixture, Harness, HelperQuery, OPENAI_MODELS, ParameterAnswer, Reply, ScriptedDynamic,
    ScriptedParameters, State, field, full_call, helper_call, number, openai_capabilities, raw,
    request, set_field, text,
};

pub(super) async fn test_full_query_order_case() {
    let fixture = Fixture::new(OPENAI_MODELS);
    let harness = fixture.harness();
    test_real_parameter_case(&harness).await;
    test_inferred_provider_case(&harness).await;
    test_failing_parameter_case(&harness).await;
    test_dynamic_answer_case(&harness).await;
    test_absent_parameter_case(&harness).await;
    test_duplicate_parameter_case(&harness).await;
    test_unmapped_model_case(&harness).await;
}

fn openai_source<'source>(
    harness: &'source Harness<'_>,
    capabilities: &'source BTreeSet<OpenAiCapability>,
) -> ProviderParameterSource<'source> {
    ProviderParameterSource {
        resolution: &harness.resolution,
        call_type: CallType::ChatCompletion,
        config: OpenAiConfigTag::Gpt,
        capabilities,
        base_model: None,
    }
}

fn expected_parameters(model: &str, capabilities: &BTreeSet<OpenAiCapability>) -> Vec<Json> {
    let query = CapabilityQuery {
        provider: ProviderId::Openai,
        call_type: CallType::ChatCompletion,
        config: OpenAiConfigTag::Gpt,
        model: ModelProjection {
            model,
            capabilities,
        },
        base_model: None,
    };
    match supported_parameters(&query) {
        ParameterCapabilities::Listed(names) => names
            .into_iter()
            .map(|name| Json::Text(name.to_owned()))
            .collect(),
        ParameterCapabilities::Absent => panic!("openai chat parameters must be listed"),
    }
}

async fn test_real_parameter_case(harness: &Harness<'_>) {
    let capabilities = openai_capabilities();
    let source = openai_source(harness, &capabilities);
    let mut state = State::new(8, ScriptedDynamic::default());
    let handle = full_call(
        harness,
        &mut state,
        &source,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("full model info");
    assert_eq!(
        field(&handle, "supported_openai_params"),
        Some(Json::List(expected_parameters("gpt-4o", &capabilities)))
    );
}

async fn test_inferred_provider_case(harness: &Harness<'_>) {
    let capabilities = openai_capabilities();
    let source = openai_source(harness, &capabilities);
    let mut state = State::new(8, ScriptedDynamic::default());
    let handle = full_call(
        harness,
        &mut state,
        &source,
        &request("gpt-4o", None, None, None),
    )
    .await
    .expect("inferred provider");
    assert_eq!(
        field(&handle, "supported_openai_params"),
        Some(Json::List(expected_parameters("gpt-4o", &capabilities)))
    );
}

async fn test_failing_parameter_case(harness: &Harness<'_>) {
    let parameters = ScriptedParameters::new(vec![ParameterAnswer::Failure]);
    let mut state = State::new(8, ScriptedDynamic::new(vec![Reply::Value(raw("{}"))]));
    let failure = full_call(
        harness,
        &mut state,
        &parameters,
        &request("llama3.2", Some("ollama"), None, None),
    )
    .await
    .expect_err("the parameter query fails");
    assert!(matches!(failure, FullMetadataError::Parameters { .. }));
    assert!(state.dynamic.calls().is_empty());
    assert_eq!(
        parameters.calls(),
        vec![("llama3.2".to_owned(), Some("ollama".to_owned()))]
    );
    assert_eq!(state.caches.full.cache_info().misses, 1);
    assert_eq!(state.caches.full.cache_info().currsize, 0);
}

// the helper's not-mapped error passes through unchanged and is never cached
async fn test_unmapped_model_case(harness: &Harness<'_>) {
    let parameters = ScriptedParameters::new(vec![
        ParameterAnswer::Answer(None),
        ParameterAnswer::Answer(None),
    ]);
    let mut state = State::new(8, ScriptedDynamic::default());
    for (calls, misses) in [(1_usize, 1_u64), (2, 2)] {
        let failure = full_call(
            harness,
            &mut state,
            &parameters,
            &request("unmapped-model", Some("openai"), None, None),
        )
        .await
        .expect_err("the model is not in the catalog");
        assert!(matches!(
            failure,
            FullMetadataError::Metadata(ModelMetadataError::NotMapped { .. })
        ));
        assert!(state.dynamic.calls().is_empty());
        assert_eq!(parameters.calls().len(), calls);
        let info = state.caches.full.cache_info();
        assert_eq!(info.misses, misses);
        assert_eq!(info.hits, 0);
        assert_eq!(info.currsize, 0);
    }
}

async fn test_dynamic_answer_case(harness: &Harness<'_>) {
    let parameters = ScriptedParameters::new(vec![ParameterAnswer::Answer(Some(vec![
        "temperature".to_owned(),
    ]))]);
    let mut state = State::new(
        8,
        ScriptedDynamic::new(vec![Reply::Value(raw(r#"{"key":"runtime"}"#))]),
    );
    let handle = full_call(
        harness,
        &mut state,
        &parameters,
        &request("llama3.2", Some("ollama"), Some("http://fixture:1"), None),
    )
    .await
    .expect("dynamic metadata");
    assert_eq!(field(&handle, "key"), Some(text("runtime")));
    assert_eq!(
        field(&handle, "supported_openai_params"),
        Some(Json::List(vec![text("temperature")]))
    );
    let calls = state.dynamic.calls();
    let call = calls.first().expect("one dynamic call");
    assert_eq!(call.adapter, DynamicModelInfo::Ollama);
    assert_eq!(call.model, "llama3.2");
    assert_eq!(call.api_base.as_deref(), Some("http://fixture:1"));
    assert!(!call.authenticated);
}

async fn test_absent_parameter_case(harness: &Harness<'_>) {
    let parameters = ScriptedParameters::new(vec![ParameterAnswer::Answer(None)]);
    let mut state = State::new(
        8,
        ScriptedDynamic::new(vec![Reply::Value(raw(r#"{"key":"runtime"}"#))]),
    );
    let handle = full_call(
        harness,
        &mut state,
        &parameters,
        &request("llama3.2", Some("ollama"), None, None),
    )
    .await
    .expect("dynamic metadata");
    assert_eq!(field(&handle, "supported_openai_params"), Some(Json::Null));
}

async fn test_duplicate_parameter_case(harness: &Harness<'_>) {
    let parameters = ScriptedParameters::new(vec![ParameterAnswer::Answer(Some(Vec::new()))]);
    let mut state = State::new(
        8,
        ScriptedDynamic::new(vec![Reply::Value(raw(
            r#"{"key":"runtime","supported_openai_params":[]}"#,
        ))]),
    );
    let failure = full_call(
        harness,
        &mut state,
        &parameters,
        &request("llama3.2", Some("ollama"), None, None),
    )
    .await
    .expect_err("the duplicate field fails construction");
    assert!(matches!(
        failure,
        FullMetadataError::Record(ModelRecordError::DuplicateSupportedParameters)
    ));
}

pub(super) async fn test_provider_overlay_case() {
    let fixture = Fixture::new(
        r#"{
        "xai/grok-2": {"litellm_provider": "xai", "mode": "chat", "supports_web_search": false},
        "fireworks_ai/llama-v3p1-8b": {"litellm_provider": "fireworks_ai", "mode": "chat", "supports_function_calling": false, "supports_prompt_caching": false},
        "llama3.2": {"litellm_provider": "ollama", "mode": "chat"}
    }"#,
    );
    let harness = fixture.harness();
    test_xai_overlay_case(&harness).await;
    test_fireworks_overlay_case(&harness).await;
    test_overlay_after_helper_case(&harness).await;
}

async fn test_xai_overlay_case(harness: &Harness<'_>) {
    let parameters = ScriptedParameters::default();
    let mut state = State::new(8, ScriptedDynamic::default());
    let full = full_call(
        harness,
        &mut state,
        &parameters,
        &request("grok-2", Some("xai"), None, None),
    )
    .await
    .expect("xai full info");
    assert_eq!(field(&full, "supports_web_search"), Some(Json::Bool(true)));
    let helper = helper_call(
        harness,
        &mut state,
        &HelperQuery {
            model: "grok-2",
            custom_llm_provider: Some("xai"),
            api_base: None,
        },
    )
    .await
    .expect("xai helper info");
    assert_eq!(
        field(&helper, "supports_web_search"),
        Some(Json::Bool(false))
    );
}

async fn test_fireworks_overlay_case(harness: &Harness<'_>) {
    let parameters = ScriptedParameters::default();
    let mut state = State::new(8, ScriptedDynamic::default());
    let full = full_call(
        harness,
        &mut state,
        &parameters,
        &request("llama-v3p1-8b", Some("fireworks_ai"), None, None),
    )
    .await
    .expect("fireworks full info");
    assert_eq!(
        field(&full, "supports_function_calling"),
        Some(Json::Bool(false))
    );
    assert_eq!(
        field(&full, "supports_prompt_caching"),
        Some(Json::Bool(true))
    );
    let helper = helper_call(
        harness,
        &mut state,
        &HelperQuery {
            model: "llama-v3p1-8b",
            custom_llm_provider: Some("fireworks_ai"),
            api_base: None,
        },
    )
    .await
    .expect("fireworks helper info");
    assert_eq!(
        field(&helper, "supports_prompt_caching"),
        Some(Json::Bool(false))
    );
}

// the overlay happens after a successful helper lookup, never instead of it
async fn test_overlay_after_helper_case(harness: &Harness<'_>) {
    let parameters = ScriptedParameters::default();
    let mut state = State::new(
        8,
        ScriptedDynamic::new(vec![Reply::Value(raw(
            r#"{"key":"runtime","supported_openai_params":[]}"#,
        ))]),
    );
    let failure = full_call(
        harness,
        &mut state,
        &parameters,
        &request("llama3.2", Some("ollama"), None, None),
    )
    .await
    .expect_err("duplicate parameter field");
    assert!(matches!(
        failure,
        FullMetadataError::Record(ModelRecordError::DuplicateSupportedParameters)
    ));
}

pub(super) async fn test_credential_cache_bypass_case() {
    let fixture = Fixture::new(OPENAI_MODELS);
    let harness = fixture.harness();
    let parameters = ScriptedParameters::default();
    let mut state = State::new(8, ScriptedDynamic::default());
    let cached = test_cache_fill_case(&harness, &mut state, &parameters).await;
    test_cache_hit_case(&harness, &mut state, &parameters, &cached).await;
    test_empty_key_case(&harness, &mut state, &parameters, &cached).await;
    test_named_key_case(&harness, &mut state, &parameters, &cached).await;
    test_api_base_key_case(&harness, &mut state, &parameters, &cached).await;
    test_explicit_dynamic_key_case(&harness).await;
}

async fn test_cache_fill_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
) -> super::ModelInfoHandle {
    let handle = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("cached lookup");
    assert_eq!(state.caches.full.cache_info().misses, 1);
    assert_eq!(state.caches.full.cache_info().currsize, 1);
    handle
}

async fn test_cache_hit_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    cached: &super::ModelInfoHandle,
) {
    let hit = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("cache hit");
    assert!(Arc::ptr_eq(cached, &hit));
    assert_eq!(state.caches.full.cache_info().hits, 1);
}

// an empty explicit key still takes the uncached build path
async fn test_empty_key_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    cached: &super::ModelInfoHandle,
) {
    let empty_key = SecretString::from(String::new());
    let explicit = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, Some(&empty_key)),
    )
    .await
    .expect("explicit empty key");
    assert!(!Arc::ptr_eq(cached, &explicit));
    assert_eq!(state.caches.full.cache_info().hits, 1);
    assert_eq!(state.caches.full.cache_info().misses, 1);
    assert_eq!(parameters.calls().len(), 2);
}

async fn test_named_key_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    cached: &super::ModelInfoHandle,
) {
    let named_key = SecretString::from("fixture-key".to_owned());
    let explicit = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, Some(&named_key)),
    )
    .await
    .expect("explicit key");
    assert!(!Arc::ptr_eq(cached, &explicit));
    assert_eq!(state.caches.full.cache_info().misses, 1);
}

// None and the empty string stay distinct cache keys
async fn test_api_base_key_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    cached: &super::ModelInfoHandle,
) {
    let missing_base = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), Some(""), None),
    )
    .await
    .expect("empty api_base");
    assert!(!Arc::ptr_eq(cached, &missing_base));
    assert_eq!(state.caches.full.cache_info().misses, 2);
    let hit = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("cache hit");
    assert!(Arc::ptr_eq(cached, &hit));
    assert_eq!(state.caches.full.cache_info().hits, 2);
}

// an explicit key still reaches the dynamic metadata query, which must receive it
async fn test_explicit_dynamic_key_case(harness: &Harness<'_>) {
    let parameters = ScriptedParameters::default();
    let mut state = State::new(
        8,
        ScriptedDynamic::new(vec![
            Reply::Value(raw(r#"{"key":"runtime"}"#)),
            Reply::Value(raw(r#"{"key":"runtime"}"#)),
        ]),
    );
    test_explicit_key(harness, &mut state, &parameters, String::new()).await;
    test_explicit_key(harness, &mut state, &parameters, "fixture-key".to_owned()).await;
    let calls = state.dynamic.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| call.authenticated));
    assert!(
        calls
            .iter()
            .all(|call| call.adapter == DynamicModelInfo::Ollama && call.model == "llama3.2")
    );
    // explicit keys read and write neither cache
    let full = state.caches.full.cache_info();
    assert_eq!(full.hits, 0);
    assert_eq!(full.misses, 0);
    assert_eq!(full.currsize, 0);
}

async fn test_explicit_key(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    key: String,
) {
    let credential = SecretString::from(key);
    let record = full_call(
        harness,
        state,
        parameters,
        &request("llama3.2", Some("ollama"), None, Some(&credential)),
    )
    .await
    .expect("explicit key");
    assert_eq!(field(&record, "key"), Some(text("runtime")));
}

pub(super) async fn test_helper_and_full_caches_case() {
    let fixture = Fixture::new(OPENAI_MODELS);
    let harness = fixture.harness();
    let parameters = ScriptedParameters::default();
    let mut state = State::new(2, ScriptedDynamic::default());
    test_helper_hit_case(&harness, &mut state).await;
    test_helper_eviction_case(&harness, &mut state).await;
    test_separate_full_cache_case(&harness, &mut state, &parameters).await;
    for maxsize in [0_i64, -3] {
        test_zero_capacity_case(&harness, maxsize, &parameters).await;
    }
    test_failed_build_case(&harness).await;
    test_huge_maxsize_case();
}

async fn test_helper_hit_case(harness: &Harness<'_>, state: &mut State) {
    let first = helper_call(
        harness,
        state,
        &HelperQuery {
            model: "gpt-4o",
            custom_llm_provider: Some("openai"),
            api_base: None,
        },
    )
    .await
    .expect("helper miss");
    assert_eq!(state.caches.helper.cache_info().misses, 1);
    assert_eq!(state.caches.helper.cache_info().maxsize, 2);
    let second = helper_call(
        harness,
        state,
        &HelperQuery {
            model: "gpt-4o",
            custom_llm_provider: Some("openai"),
            api_base: None,
        },
    )
    .await
    .expect("helper hit");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(state.caches.helper.cache_info().hits, 1);
}

async fn test_helper_eviction_case(harness: &Harness<'_>, state: &mut State) {
    for model in ["gpt-4", "gpt-3.5-turbo"] {
        drop(
            helper_call(
                harness,
                state,
                &HelperQuery {
                    model,
                    custom_llm_provider: Some("openai"),
                    api_base: None,
                },
            )
            .await
            .expect("helper fill"),
        );
    }
    assert_eq!(state.caches.helper.cache_info().currsize, 2);
    assert_eq!(state.caches.helper.cache_info().misses, 3);
    // gpt-4o was the least recently used entry, so the third model evicted it
    drop(
        helper_call(
            harness,
            state,
            &HelperQuery {
                model: "gpt-4o",
                custom_llm_provider: Some("openai"),
                api_base: None,
            },
        )
        .await
        .expect("evicted helper entry"),
    );
    assert_eq!(state.caches.helper.cache_info().misses, 4);
}

async fn test_separate_full_cache_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
) {
    drop(
        full_call(
            harness,
            state,
            parameters,
            &request("gpt-4o", Some("openai"), None, None),
        )
        .await
        .expect("full miss"),
    );
    assert_eq!(state.caches.full.cache_info().misses, 1);
    assert_eq!(state.caches.full.cache_info().currsize, 1);
    assert_eq!(state.caches.helper.cache_info().misses, 4);
    state.caches.full.cache_clear();
    assert_eq!(
        state.caches.full.cache_info(),
        CacheInfo {
            hits: 0,
            misses: 0,
            maxsize: 2,
            currsize: 0,
        }
    );
    assert_eq!(state.caches.helper.cache_info().misses, 4);
}

async fn test_zero_capacity_case(
    harness: &Harness<'_>,
    maxsize: i64,
    parameters: &ScriptedParameters,
) {
    let mut state = State::new(maxsize, ScriptedDynamic::default());
    for _attempt in 0_u8..2 {
        drop(
            full_call(
                harness,
                &mut state,
                parameters,
                &request("gpt-4o", Some("openai"), None, None),
            )
            .await
            .expect("uncached lookup"),
        );
    }
    let info = state.caches.full.cache_info();
    assert_eq!(info.maxsize, 0);
    assert_eq!(info.hits, 0);
    assert_eq!(info.misses, 2);
    assert_eq!(info.currsize, 0);
}

// functools.lru_cache allocates nothing up front, so a huge maxsize must not either
fn test_huge_maxsize_case() {
    let cache = ModelInfoCache::new(i64::MAX);
    let info = cache.cache_info();
    assert_eq!(info.maxsize, usize::try_from(i64::MAX).unwrap_or(0));
    assert_eq!(info.currsize, 0);
    let key = CacheKey {
        model: "gpt-4o".to_owned(),
        custom_llm_provider: Some("openai".to_owned()),
        api_base: None,
    };
    let record = Arc::new(Mutex::new(ModelInfo {
        fields: IndexMap::new(),
    }));
    cache.put(key.clone(), Arc::clone(&record));
    let hit = cache.get(&key).expect("huge capacity entry");
    assert!(Arc::ptr_eq(&record, &hit));
    let filled = cache.cache_info();
    assert_eq!(filled.hits, 1);
    assert_eq!(filled.misses, 0);
    assert_eq!(filled.currsize, 1);
}

// a failed build is never cached
async fn test_failed_build_case(harness: &Harness<'_>) {
    let mut state = State::new(2, ScriptedDynamic::default());
    let failures = ScriptedParameters::new(vec![
        ParameterAnswer::Failure,
        ParameterAnswer::Failure,
        ParameterAnswer::Answer(None),
    ]);
    for _attempt in 0_u8..2 {
        drop(
            full_call(
                harness,
                &mut state,
                &failures,
                &request("gpt-4o", Some("openai"), None, None),
            )
            .await
            .expect_err("the parameter query fails"),
        );
    }
    assert_eq!(state.caches.full.cache_info().misses, 2);
    assert_eq!(state.caches.full.cache_info().currsize, 0);
    drop(
        full_call(
            harness,
            &mut state,
            &failures,
            &request("gpt-4o", Some("openai"), None, None),
        )
        .await
        .expect("successful build"),
    );
    assert_eq!(state.caches.full.cache_info().misses, 3);
    assert_eq!(state.caches.full.cache_info().currsize, 1);
}

pub(super) async fn test_invalidation_generation_case() {
    let mut fixture = Fixture::new(
        r#"{"gpt-4o": {"litellm_provider": "openai", "mode": "chat", "max_tokens": 100}}"#,
    );
    let parameters = ScriptedParameters::default();
    let mut state = State::new(8, ScriptedDynamic::default());
    let key = HelperQuery {
        model: "gpt-4o",
        custom_llm_provider: Some("openai"),
        api_base: None,
    };
    let first_harness = fixture.harness();
    let original = test_first_lookup_case(&first_harness, &mut state, &parameters, &key).await;
    drop(first_harness);

    // an in-place value replacement keeps the catalog length and identity
    fixture.replace(
        "gpt-4o",
        r#"{"litellm_provider": "openai", "mode": "chat", "max_tokens": 200}"#,
    );
    let second_harness = fixture.harness();
    test_stable_before_invalidation_case(&second_harness, &mut state, &parameters, &key, &original)
        .await;
    drop(second_harness);
    test_invalidate_case(&mut state);
    let third_harness = fixture.harness();
    test_fresh_after_invalidation_case(&third_harness, &mut state, &parameters, &key).await;
    test_reload_instance_case(&third_harness, &parameters).await;
}

async fn test_first_lookup_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    key: &HelperQuery<'_>,
) -> super::ModelInfoHandle {
    let original = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("first lookup");
    drop(helper_call(harness, state, key).await.expect("helper"));
    assert_eq!(field(&original, "max_tokens"), Some(number(100)));
    assert_eq!(state.keys.generation, 0);
    original
}

async fn test_stable_before_invalidation_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    key: &HelperQuery<'_>,
    original: &super::ModelInfoHandle,
) {
    let stable = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("cache hit before invalidation");
    assert!(Arc::ptr_eq(original, &stable));
    assert_eq!(field(&stable, "max_tokens"), Some(number(100)));
    let stable_helper = helper_call(harness, state, key)
        .await
        .expect("helper hit before invalidation");
    assert_eq!(field(&stable_helper, "max_tokens"), Some(number(100)));
    assert_eq!(state.caches.full.cache_info().hits, 1);
    assert_eq!(state.caches.helper.cache_info().hits, 1);
}

fn test_invalidate_case(state: &mut State) {
    state
        .keys
        .invalidate(&mut state.caches.full, &mut state.caches.helper);
    assert_eq!(state.keys.generation, 1);
    assert_eq!(state.caches.full.cache_info().currsize, 0);
    assert_eq!(state.caches.helper.cache_info().currsize, 0);
    assert_eq!(state.caches.full.cache_info().hits, 0);
    assert_eq!(state.caches.helper.cache_info().hits, 0);
}

async fn test_fresh_after_invalidation_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    key: &HelperQuery<'_>,
) {
    let fresh = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("fresh full lookup");
    assert_eq!(field(&fresh, "max_tokens"), Some(number(200)));
    let fresh_helper = helper_call(harness, state, key)
        .await
        .expect("fresh helper lookup");
    assert_eq!(field(&fresh_helper, "max_tokens"), Some(number(200)));
    state
        .keys
        .invalidate(&mut state.caches.full, &mut state.caches.helper);
    assert_eq!(state.keys.generation, 2);
}

// a reload builds a fresh cache instance instead of reusing the old one
async fn test_reload_instance_case(harness: &Harness<'_>, parameters: &ScriptedParameters) {
    let mut reloaded = State::new(8, ScriptedDynamic::default());
    drop(
        full_call(
            harness,
            &mut reloaded,
            parameters,
            &request("gpt-4o", Some("openai"), None, None),
        )
        .await
        .expect("fresh instance lookup"),
    );
    assert_eq!(reloaded.caches.full.cache_info().misses, 1);
    assert_eq!(reloaded.caches.full.cache_info().currsize, 1);
    assert_eq!(reloaded.keys.generation, 0);
}

pub(super) async fn test_cached_record_mutation_case() {
    let mut fixture = Fixture::new(
        r#"{"gpt-4o": {"litellm_provider": "openai", "mode": "chat", "max_tokens": 100}}"#,
    );
    let parameters = ScriptedParameters::default();
    let mut state = State::new(8, ScriptedDynamic::default());
    let key = HelperQuery {
        model: "gpt-4o",
        custom_llm_provider: Some("openai"),
        api_base: None,
    };
    let first_harness = fixture.harness();
    let record = test_mutated_first_record_case(&first_harness, &mut state, &parameters).await;
    test_mutation_hit_case(&first_harness, &mut state, &parameters, &record).await;
    test_explicit_record_case(&first_harness, &mut state, &parameters, &record).await;
    test_rebuilt_record_case(&first_harness, &mut state, &parameters, &record).await;
    let helper = test_helper_mutation_case(&first_harness, &mut state, &key).await;
    drop(first_harness);

    fixture.replace(
        "gpt-4o",
        r#"{"litellm_provider": "openai", "mode": "chat", "max_tokens": 300}"#,
    );
    state
        .keys
        .invalidate(&mut state.caches.full, &mut state.caches.helper);
    let second_harness = fixture.harness();
    test_fresh_helper_case(&second_harness, &mut state, &key, &helper).await;
}

async fn test_mutated_first_record_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
) -> super::ModelInfoHandle {
    let record = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("first lookup");
    set_field(&record, "max_tokens", number(123));
    record
}

async fn test_mutation_hit_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    record: &super::ModelInfoHandle,
) {
    let hit = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("cache hit");
    assert!(Arc::ptr_eq(record, &hit));
    assert_eq!(field(&hit, "max_tokens"), Some(number(123)));
}

// an explicit-key call builds a fresh record and never touches the cached one
async fn test_explicit_record_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    record: &super::ModelInfoHandle,
) {
    let named_key = SecretString::from("fixture-key".to_owned());
    let explicit = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, Some(&named_key)),
    )
    .await
    .expect("explicit key");
    assert!(!Arc::ptr_eq(record, &explicit));
    assert_eq!(field(&explicit, "max_tokens"), Some(number(100)));
    set_field(&explicit, "max_tokens", number(999));
    assert_eq!(field(record, "max_tokens"), Some(number(123)));
}

async fn test_rebuilt_record_case(
    harness: &Harness<'_>,
    state: &mut State,
    parameters: &ScriptedParameters,
    record: &super::ModelInfoHandle,
) {
    state.caches.full.cache_clear();
    let rebuilt = full_call(
        harness,
        state,
        parameters,
        &request("gpt-4o", Some("openai"), None, None),
    )
    .await
    .expect("rebuilt lookup");
    assert!(!Arc::ptr_eq(record, &rebuilt));
    assert_eq!(field(&rebuilt, "max_tokens"), Some(number(100)));
}

// the helper cache shares its record the same way
async fn test_helper_mutation_case(
    harness: &Harness<'_>,
    state: &mut State,
    key: &HelperQuery<'_>,
) -> super::ModelInfoHandle {
    let helper = helper_call(harness, state, key)
        .await
        .expect("helper lookup");
    set_field(&helper, "max_tokens", number(7));
    let helper_hit = helper_call(harness, state, key).await.expect("helper hit");
    assert!(Arc::ptr_eq(&helper, &helper_hit));
    assert_eq!(field(&helper_hit, "max_tokens"), Some(number(7)));
    helper
}

async fn test_fresh_helper_case(
    harness: &Harness<'_>,
    state: &mut State,
    key: &HelperQuery<'_>,
    helper: &super::ModelInfoHandle,
) {
    let fresh = helper_call(harness, state, key)
        .await
        .expect("fresh helper lookup");
    assert!(!Arc::ptr_eq(helper, &fresh));
    assert_eq!(field(&fresh, "max_tokens"), Some(number(300)));
}
