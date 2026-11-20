use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::{BackendError, CacheResult, Ttl};
use betterlite_cache::cache_key::settings::CacheKeySettings;
use betterlite_cache::envelope::backend::EnvelopeBackend;
use betterlite_cache::envelope::facade::{EnvelopeSettings, ResponseCache};
use betterlite_cache::envelope::lookup::CacheLookup;
use betterlite_cache::types::cache_type::{CacheMode, LiteLlmCacheType};
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_logging::verbose_hooks::{ConsoleSink, LoggerSink, VerboseHooks};
use betterlite_models::decisions::Json;
use betterlite_models::key_digest::hashing::hash_token;
use betterlite_redact::secret::Redactor;
use betterlite_redact::structured::Payload;

use super::{SpendCache as _, SpendResponse, kwargs, metadata_of, object, run, settings};

struct KeylessBackend;

#[async_trait]
impl EnvelopeBackend for KeylessBackend {
    fn get_cache(&self, _key: &str, _lookup: &mut CacheLookup) -> CacheResult<Option<CacheValue>> {
        Err(BackendError::Unsupported("get_cache"))
    }

    fn set_cache(&self, _key: &str, _value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        Err(BackendError::Unsupported("set_cache"))
    }

    async fn async_get_cache(
        &self,
        _key: &str,
        _lookup: &mut CacheLookup,
    ) -> CacheResult<Option<CacheValue>> {
        Err(BackendError::Unsupported("async_get_cache"))
    }

    async fn async_set_cache(&self, _key: &str, _value: CacheValue, _ttl: Ttl) -> CacheResult<()> {
        Err(BackendError::Unsupported("async_set_cache"))
    }

    async fn batch_cache_write(
        &self,
        _key: &str,
        _value: CacheValue,
        _ttl: Ttl,
    ) -> CacheResult<()> {
        Err(BackendError::Unsupported("batch_cache_write"))
    }
}

struct NullLogger;

impl LoggerSink for NullLogger {
    fn debug(&self, _payload: &Payload) -> bool {
        false
    }

    fn info(&self, _payload: &Payload) -> bool {
        false
    }

    fn error(&self, _payload: &Payload) -> bool {
        false
    }

    fn debug_enabled(&self) -> bool {
        false
    }
}

struct NullConsole;

impl ConsoleSink for NullConsole {
    fn print(&self, _payload: &Payload) -> bool {
        false
    }
}

fn response_cache() -> ResponseCache<NullLogger, NullConsole> {
    ResponseCache {
        backend: Arc::new(KeylessBackend),
        key_settings: CacheKeySettings {
            cache_type: LiteLlmCacheType::Local,
            mode: CacheMode::DefaultOff,
            namespace: None,
            semantic_cache_scope: SemanticCacheScope::Key,
            include_provider_params: false,
        },
        settings: EnvelopeSettings {
            ttl: None,
            redis_flush_size: None,
        },
        hooks: VerboseHooks {
            logger: NullLogger,
            console: NullConsole,
            set_verbose: Payload::Bool(false),
            redactor: Arc::new(Redactor::new(8).expect("redactor")),
        },
        clock: Arc::new(|| 0.0_f64),
    }
}

#[test]
fn bodies_store_nothing_when_body_storage_is_off() {
    let payload =
        run(r#"{"model": "gpt-4o"}"#, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.response, "{}");
    assert_eq!(payload.proxy_server_request, "{}");
}

#[test]
fn a_placeholdered_request_rewrites_the_stored_body_model() {
    let text = r#"{"model": "tell me a story",
        "litellm_params": {"proxy_server_request": {"body": {
            "model": "tell me a story",
            "metadata": {"model_group": "tell me a story"}
        }}}}"#;
    let mut bodies = settings();
    bodies.storage.sanitize.store_bodies = true;
    let payload = run(text, SpendResponse::UnknownModelError, &bodies).expect("payload");
    assert!(payload.proxy_server_request.contains("unknown-model"));
    assert!(!payload.proxy_server_request.contains("tell me a story"));
}

#[test]
fn a_trusted_hash_keeps_the_key_that_matches_it() {
    let value = "a".repeat(64);
    let text = format!(
        r#"{{"model": "m", "litellm_params": {{"metadata": {{
            "user_api_key": "{value}",
            "user_api_key_hash": "{value}"
        }}}}}}"#
    );
    let payload = run(&text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.api_key, value);
}

#[test]
fn a_trusted_hash_of_another_value_still_hashes_the_key() {
    let value = "a".repeat(64);
    let text = format!(
        r#"{{"model": "m", "litellm_params": {{"metadata": {{
            "user_api_key": "{value}",
            "user_api_key_hash": "litellm_proxy_master_key"
        }}}}}}"#
    );
    let payload = run(&text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.api_key, hash_token(&value));
}

#[test]
fn the_standard_payload_supplies_a_trusted_key_hash() {
    let value = "b".repeat(64);
    let text = format!(
        r#"{{"model": "m", "standard_logging_object": {{"metadata": {{"user_api_key_hash": "{value}"}}}}}}"#
    );
    let payload = run(&text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.api_key, value);
}

#[test]
fn an_empty_end_user_falls_to_the_standard_payload() {
    let text = r#"{"model": "m",
        "litellm_params": {"metadata": {"user_api_key_end_user_id": ""}},
        "standard_logging_object": {"metadata": {"user_api_key_end_user_id": "stored"}}}"#;
    let payload = run(text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.end_user, "stored");
}

#[test]
fn a_null_standard_tag_list_falls_to_metadata_tags() {
    let text = r#"{"model": "m",
        "litellm_params": {"metadata": {"tags": ["alpha"]}},
        "standard_logging_object": {"request_tags": null}}"#;
    let payload = run(text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.request_tags, r#"["alpha"]"#);
}

#[test]
fn empty_attribution_values_fall_to_the_standard_payload() {
    let text = r#"{"model": "gpt-4o",
        "litellm_params": {"api_base": "", "metadata": {"model_info": {"id": ""}, "model_group": ""}},
        "standard_logging_object": {"model_id": "model-uuid", "model_group": "group", "api_base": "https://standard.example"}}"#;
    let payload = run(text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.model_id, "model-uuid");
    assert_eq!(payload.model_group, "group");
    assert_eq!(payload.api_base, "https://standard.example");
}

#[test]
fn an_empty_provider_kwarg_falls_to_the_standard_payload() {
    let payload = run(
        r#"{"model": "gpt-4o", "custom_llm_provider": "", "standard_logging_object": {"custom_llm_provider": "openai"}}"#,
        SpendResponse::Absent,
        &settings(),
    )
    .expect("payload");
    assert_eq!(payload.custom_llm_provider, "openai");
}

#[test]
fn an_empty_standard_model_falls_to_the_requested_model() {
    let payload = run(
        r#"{"model": "gpt-4o", "standard_logging_object": {"model": ""}}"#,
        SpendResponse::Absent,
        &settings(),
    )
    .expect("payload");
    assert_eq!(payload.model, "gpt-4o");
}

#[test]
fn an_empty_call_id_kwarg_falls_to_the_params() {
    let payload = run(
        r#"{"model": "m", "litellm_call_id": "", "litellm_params": {"litellm_call_id": "params-call"}}"#,
        SpendResponse::Absent,
        &settings(),
    )
    .expect("payload");
    assert_eq!(payload.litellm_call_id.as_deref(), Some("params-call"));
}

#[test]
fn a_placeholdered_group_that_differs_from_the_raw_model_survives() {
    let text = r#"{"model": "tell me a story",
        "litellm_params": {"metadata": {"model_group": "safe-group"}}}"#;
    let payload = run(text, SpendResponse::UnknownModelError, &settings()).expect("payload");
    assert_eq!(payload.model, "unknown-model");
    assert_eq!(payload.model_group, "safe-group");
}

#[test]
fn an_unsafe_group_survives_when_the_model_is_not_placeholdered() {
    let text = r#"{"model": "tell me a story",
        "litellm_params": {"metadata": {"model_group": "tell me a story"}}}"#;
    let payload = run(text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.model, "tell me a story");
    assert_eq!(payload.model_group, "tell me a story");
}

#[test]
fn the_oauth_stamp_is_forwarded_only_for_anthropic() {
    let text = r#"{"model": "gpt-4o",
        "custom_llm_provider": "openai",
        "litellm_params": {"metadata": {"used_client_oauth_token": true}}}"#;
    let payload = run(text, SpendResponse::Absent, &settings()).expect("payload");
    let metadata = metadata_of(&payload);
    assert_eq!(
        metadata.get("used_client_oauth_token"),
        Some(&Json::Bool(false))
    );
}

#[test]
fn token_totals_are_not_additional_usage_values() {
    let response = kwargs(
        r#"{"usage": {"total_tokens": 5, "prompt_tokens": 3, "completion_tokens": 2, "pages": 1}}"#,
    );
    let payload = run(
        r#"{"model": "m"}"#,
        SpendResponse::Mapping(&response),
        &settings(),
    )
    .expect("payload");
    let metadata = metadata_of(&payload);
    let additional = object(
        metadata
            .get("additional_usage_values")
            .expect("usage values"),
    );
    assert!(additional.contains_key("pages"));
    assert!(!additional.contains_key("total_tokens"));
    assert!(!additional.contains_key("prompt_tokens"));
    assert!(!additional.contains_key("completion_tokens"));
}

#[test]
fn the_configured_cache_derives_the_key_from_the_request() {
    let cache = response_cache();
    let first = cache
        .cache_key(&kwargs(r#"{"model": "gpt-4o"}"#))
        .expect("derived key");
    let other = cache
        .cache_key(&kwargs(r#"{"model": "gpt-4o-mini"}"#))
        .expect("derived key");
    assert_ne!(first, "xyzzy");
    assert!(!first.is_empty());
    assert_ne!(first, other);
}
