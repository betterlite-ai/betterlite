#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_config::collector::TracebackPolicy;
use betterlite_config::spend_attribution::Settings as SpendAttributionSettings;
use betterlite_config::spend_storage::MissingSessionId;
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::payload::deps::PayloadClock;
use betterlite_logging::spend_bodies::SpendStoragePolicy;
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_logging::spend_metadata::{SpendLogStatus, SpendLogsPayload};
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::key_digest::hashing::hash_token;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::Number;
use tokio::sync::watch;

use super::{
    CompletionStart, PayloadRequest, SpendCache, SpendPayloadError, SpendPayloadSettings,
    SpendResponse, SpendRouter, get_logging_payload,
};

struct Clock(f64);

impl PayloadClock for Clock {
    fn now_epoch_seconds(&self) -> f64 {
        self.0
    }
}

struct DerivedCache;

impl SpendCache for DerivedCache {
    fn cache_key(&self, kwargs: &JsonObject) -> Result<String, SpendPayloadError> {
        assert!(kwargs.contains_key("model"));
        Ok("derived-key".to_owned())
    }
}

struct Catalog {
    catalog: Arc<ModelCatalog>,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Catalog {
    fn new() -> Self {
        Self {
            catalog: BundledCatalog::default().get(),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

fn settings() -> SpendPayloadSettings {
    SpendPayloadSettings {
        storage: SpendStoragePolicy {
            sanitize: SpendSanitizePolicy {
                store_bodies: false,
                max_string_length: 2048,
            },
            turn_off_message_logging: false,
        },
        missing_session_id: None,
        end_user: EndUserCostTrackingSettings::default(),
    }
}

fn timestamp(text: &str) -> SpendTimestamp {
    SpendTimestamp::Aware(DateTime::parse_from_rfc3339(text).expect("timestamp"))
}

fn kwargs(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("kwargs")
}

struct Harness<'request> {
    kwargs: JsonObject,
    router: Option<&'request SpendRouter<'request>>,
    cache: Option<&'request dyn SpendCache>,
}

impl<'request> Harness<'request> {
    fn new(text: &str) -> Self {
        Self {
            kwargs: kwargs(text),
            router: None,
            cache: None,
        }
    }

    const fn with_router(mut self, router: &'request SpendRouter<'request>) -> Self {
        self.router = Some(router);
        self
    }

    const fn with_cache(mut self, cache: &'request dyn SpendCache) -> Self {
        self.cache = Some(cache);
        self
    }

    fn request(
        &'request self,
        response: SpendResponse<'request>,
        clock: &'request Clock,
        tracebacks: &'request watch::Receiver<TracebackPolicy>,
    ) -> PayloadRequest<'request> {
        PayloadRequest {
            kwargs: &self.kwargs,
            response,
            start_time: timestamp("2024-01-01T00:00:00Z"),
            end_time: timestamp("2024-01-01T00:00:02.500Z"),
            completion_start_time: CompletionStart::Absent,
            router: self.router,
            cache: self.cache,
            clock,
            session_uuid: "00000000-0000-0000-0000-000000000001",
            tracebacks,
        }
    }

    fn run(
        &'request self,
        response: SpendResponse<'request>,
        settings: &SpendPayloadSettings,
    ) -> Result<SpendLogsPayload, SpendPayloadError> {
        let clock = Clock(1_700_000_000.5);
        let (_sender, tracebacks) = watch::channel(TracebackPolicy { suppress: false });
        get_logging_payload(&self.request(response, &clock, &tracebacks), settings, &Utc)
    }
}

fn run(
    text: &str,
    response: SpendResponse<'_>,
    settings: &SpendPayloadSettings,
) -> Result<SpendLogsPayload, SpendPayloadError> {
    Harness::new(text).run(response, settings)
}

fn metadata_of(payload: &SpendLogsPayload) -> JsonObject {
    let value: Json = serde_json::from_str(&payload.metadata).expect("metadata json");
    match value {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("metadata is an object")
        }
    }
}

fn object(value: &Json) -> &JsonObject {
    match value {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("value is an object")
        }
    }
}

fn number(value: &Json) -> i64 {
    match value {
        Json::Number(number) => number.as_i64().expect("integer"),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            panic!("value is an integer")
        }
    }
}

const STANDARD: &str = r#"
    "standard_logging_object": {
        "id": "standard-id",
        "model": "openai/gpt-4o",
        "model_id": "model-uuid",
        "model_group": "group",
        "api_base": "https://standard.example",
        "custom_llm_provider": "openai",
        "request_tags": ["beta"],
        "hidden_params": {
            "batch_models": ["batched"],
            "batch_successful_requests": 2,
            "batch_failed_requests": 1,
            "litellm_overhead_time_ms": 12.5,
            "additional_headers": {"llm_provider-x-ms-is-spilled-over": "true"}
        },
        "metadata": {
            "applied_guardrails": ["guard"],
            "cold_storage_object_key": "cold/key",
            "usage_object": {"prompt_tokens_details": {"cache_creation_tokens": 9}}
        },
        "guardrail_information": [{"guardrail_name": "g"}],
        "cost_breakdown": {"input_cost": 0.1},
        "autorouter_savings": 0.02,
        "autorouter_baseline_observation": "baseline"
    }
"#;

#[test]
fn assembles_a_standard_enriched_payload() {
    let text = format!(
        r#"{{
            "model": "gpt-4o",
            "call_type": "acompletion",
            "response_cost": 0.25,
            "litellm_params": {{
                "api_base": "https://params.example",
                "litellm_call_id": "call-1",
                "metadata": {{
                    "user_api_key": "Bearer sk-secret",
                    "user_api_key_user_id": "user-1",
                    "user_api_key_team_id": "team-1",
                    "user_api_key_org_id": "org-1",
                    "tags": ["alpha"]
                }}
            }},
            {STANDARD}
        }}"#
    );
    let payload = run(&text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.request_id, "standard-id");
    assert_eq!(payload.call_type, "acompletion");
    assert_eq!(payload.api_key, hash_token("sk-secret"));
    assert_eq!(payload.model, "openai/gpt-4o");
    assert_eq!(payload.model_id, "model-uuid");
    assert_eq!(payload.model_group, "group");
    assert_eq!(payload.api_base, "https://params.example");
    assert_eq!(payload.user, "user-1");
    assert_eq!(payload.team_id, "team-1");
    assert_eq!(payload.organization_id, "org-1");
    assert_eq!(payload.custom_llm_provider, "openai");
    assert_eq!(payload.request_tags, r#"["beta"]"#);
    assert_eq!(payload.spend, Some(Decimal::new(25, 2)));
    assert_eq!(payload.status, SpendLogStatus::Success);
    assert_eq!(payload.litellm_call_id.as_deref(), Some("call-1"));
    assert_eq!(payload.request_duration_ms, Some(2500));
    let metadata = metadata_of(&payload);
    assert_eq!(
        metadata.get("applied_guardrails"),
        Some(&Json::List(vec![Json::Text("guard".to_owned())]))
    );
    assert_eq!(
        metadata.get("cold_storage_object_key"),
        Some(&Json::Text("cold/key".to_owned()))
    );
    assert_eq!(
        metadata.get("batch_successful_requests"),
        Some(&Json::Number(Number::from(2)))
    );
    assert_eq!(
        metadata.get("batch_failed_requests"),
        Some(&Json::Number(Number::from(1)))
    );
    assert!(metadata.contains_key("guardrail_information"));
    assert!(metadata.contains_key("cost_breakdown"));
    assert!(metadata.contains_key("autorouter_savings"));
    assert_eq!(
        object(metadata.get("azure_spillover").expect("spillover")).get("from_deployment"),
        Some(&Json::Null)
    );
    let additional = object(
        metadata
            .get("additional_usage_values")
            .expect("usage values"),
    );
    assert_eq!(
        additional.get("cache_creation_input_tokens"),
        Some(&Json::Number(Number::from(9)))
    );
    assert_eq!(
        payload.session_id.as_deref(),
        Some("00000000-0000-0000-0000-000000000001")
    );
}

#[test]
fn missing_fields_keep_the_sources_empty_string_defaults() {
    let payload =
        run(r#"{"model": "gpt-4o"}"#, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.api_key, "");
    assert_eq!(payload.user, "");
    assert_eq!(payload.team_id, "");
    assert_eq!(payload.organization_id, "");
    assert_eq!(payload.end_user, "");
    assert_eq!(payload.custom_llm_provider, "");
    assert_eq!(payload.model_id, "");
    assert_eq!(payload.model_group, "");
    assert_eq!(payload.api_base, "");
    assert_eq!(payload.request_tags, "[]");
    assert_eq!(payload.call_type, "");
    assert_eq!(payload.spend, Some(Decimal::ZERO));
    assert_eq!(payload.cache_key, "Cache OFF");
    assert_eq!(payload.cache_hit, "False");
    assert_eq!(payload.request_id, "None");
    assert_eq!(
        payload.session_id.as_deref(),
        Some("00000000-0000-0000-0000-000000000001")
    );
}

#[test]
fn the_caller_kwargs_are_not_mutated() {
    let cache = DerivedCache;
    let harness = Harness::new(
        r#"{"model": "gpt-4o", "litellm_params": {"metadata": {"model_group": "group"}},
            "standard_logging_object": {"model": "openai/gpt-4o"}}"#,
    )
    .with_cache(&cache);
    let before = serde_json::to_value(&harness.kwargs).expect("encoded");
    let payload = harness
        .run(SpendResponse::Absent, &settings())
        .expect("payload");
    assert_eq!(payload.cache_key, "derived-key");
    assert_eq!(
        serde_json::to_value(&harness.kwargs).expect("encoded"),
        before
    );
}

#[test]
fn settings_are_read_per_call() {
    let harness = Harness::new(
        r#"{"model": "gpt-4o", "litellm_params": {"user_api_key_end_user_id": "user"}}"#,
    );
    let disabled = SpendPayloadSettings {
        end_user: EndUserCostTrackingSettings {
            disable_end_user_cost_tracking: Some(true),
            enable_prometheus_only: None,
        },
        ..settings()
    };
    let first = harness
        .run(SpendResponse::Absent, &disabled)
        .expect("payload");
    let second = harness
        .run(SpendResponse::Absent, &settings())
        .expect("payload");
    assert_eq!(first.end_user, "");
    assert_eq!(second.end_user, "user");
}

#[test]
fn unknown_model_errors_placeholder_the_model_and_clear_an_unsafe_group() {
    let payload = run(
        r#"{"model": "tell me a story", "litellm_params": {"metadata": {"model_group": "tell me a story"}}}"#,
        SpendResponse::UnknownModelError,
        &settings(),
    )
    .expect("payload");
    assert_eq!(payload.model, "unknown-model");
    assert_eq!(payload.model_group, "");
    let metadata = metadata_of(&payload);
    assert_eq!(metadata.get("error_information"), Some(&Json::Null));
}

#[test]
fn malformed_requested_models_placeholder() {
    let payload = run(r#"{"model": 7}"#, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.model, "unknown-model");
}

#[test]
fn prompt_shaped_failures_placeholder_unless_the_group_is_configured() {
    let text = r#"{"model": "tell me a story",
        "litellm_params": {"metadata": {"status": "failure", "model_group": "tell me a story"}}}"#;
    let hidden = run(text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(hidden.model, "unknown-model");
    let mut catalog = DeploymentCatalog::default();
    catalog.push(DeploymentSpec {
        model_name: "tell me a story".to_owned(),
        litellm_params: kwargs(r#"{"model": "openai/gpt-4o"}"#),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    });
    let aliases = AliasMap::new();
    let groups = RoutingGroupIndex::default();
    let inferred = |_: &str| -> Option<String> { None };
    let lookup = CatalogLookup {
        catalog: &catalog,
        model_group_alias: &aliases,
        routing_groups: &groups,
        inferred_provider: &inferred,
    };
    let fixture = Catalog::new();
    let resolution = fixture.context();
    let attribution = SpendAttributionSettings::default();
    let recognized = BTreeSet::from(["tell me a story".to_owned()]);
    let teams = BTreeSet::new();
    let router = SpendRouter {
        catalog: Some(&lookup),
        settings: &attribution,
        resolution: &resolution,
        recognized_groups: &recognized,
        team_public_names: &teams,
    };
    let harness = Harness::new(text).with_router(&router);
    let shown = harness
        .run(SpendResponse::Absent, &settings())
        .expect("payload");
    assert_eq!(shown.model, "tell me a story");
}

#[test]
fn scrubbed_metadata_replaces_the_raw_model_in_error_information() {
    let text = r#"{"model": "tell me a story",
        "litellm_params": {"metadata": {
            "model_info": {"internal_router_model": false},
            "error_information": {"error_message": "tell me a story failed", "error_code": "400"}
        }}}"#;
    let payload = run(text, SpendResponse::UnknownModelError, &settings()).expect("payload");
    let metadata = metadata_of(&payload);
    let information = object(
        metadata
            .get("error_information")
            .expect("error information"),
    );
    assert_eq!(
        information.get("error_message"),
        Some(&Json::Text("unknown-model failed".to_owned()))
    );
    assert_eq!(
        information.get("error_code"),
        Some(&Json::Text("400".to_owned()))
    );
}

#[rstest]
#[case::true_suffix(Some(true), "None_cache_hit1700000000.5")]
#[case::false_plain(Some(false), "None")]
#[case::null_plain(None, "None")]
fn cache_hit_suffixes_follow_the_clock(#[case] hit: Option<bool>, #[case] expected: &str) {
    let encoded = match hit {
        Some(flag) => format!(r#"{{"model": "gpt-4o", "cache_hit": {flag}}}"#),
        None => r#"{"model": "gpt-4o", "cache_hit": null}"#.to_owned(),
    };
    let payload = run(&encoded, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.request_id, expected);
    assert_eq!(
        payload.cache_hit,
        if hit == Some(true) {
            "True"
        } else if hit.is_some() {
            "False"
        } else {
            "None"
        }
    );
}

#[rstest]
#[case::off("{}", None, "Cache OFF")]
#[case::derived("{}", Some("derived"), "derived-key")]
#[case::preset(r#"{"preset_cache_key": "given"}"#, Some("derived"), "given")]
#[case::empty_preset(r#"{"preset_cache_key": ""}"#, Some("derived"), "")]
#[case::null_preset(r#"{"preset_cache_key": null}"#, Some("derived"), "derived-key")]
#[case::null_preset_off(r#"{"preset_cache_key": null}"#, None, "Cache OFF")]
fn cache_keys_read_the_preset_the_cache_and_off(
    #[case] params: &str,
    #[case] cache: Option<&str>,
    #[case] expected: &str,
) {
    let text = format!(r#"{{"model": "gpt-4o", "litellm_params": {params}}}"#);
    let derived = DerivedCache;
    let plain = Harness::new(&text);
    let harness = match cache {
        Some(_) => plain.with_cache(&derived),
        None => plain,
    };
    let payload = harness
        .run(SpendResponse::Absent, &settings())
        .expect("payload");
    assert_eq!(payload.cache_key, expected);
}

#[test]
fn preset_cache_keys_must_be_text() {
    let Err(error) = run(
        r#"{"model": "gpt-4o", "litellm_params": {"preset_cache_key": 4}}"#,
        SpendResponse::Absent,
        &settings(),
    ) else {
        panic!("a non-text preset is out of type");
    };
    assert!(error.to_string().contains("preset_cache_key"));
}

#[rstest]
#[case::from_response(
    r#"{"usage": {"total_tokens": 5, "prompt_tokens": 3, "completion_tokens": 2}}"#,
    5,
    3,
    2
)]
#[case::null_wins_by_presence(r#"{"usage": {"total_tokens": null, "prompt_tokens": 3, "completion_tokens": 2}}"#, -1, 3, 2)]
#[case::missing_falls_to_standard(
    r#"{"usage": {"prompt_tokens": 3, "completion_tokens": 2}}"#,
    9,
    3,
    2
)]
fn response_usage_overrides_standard_counters_by_presence(
    #[case] response: &str,
    #[case] total: i64,
    #[case] prompt: i64,
    #[case] completion: i64,
) {
    let text = r#"{"model": "gpt-4o",
        "standard_logging_object": {"total_tokens": 9, "prompt_tokens": 8, "completion_tokens": 7}}"#;
    let message: JsonObject = serde_json::from_str(response).expect("response");
    let payload = run(&text, SpendResponse::Mapping(&message), &settings()).expect("payload");
    assert_eq!(payload.total_tokens.unwrap_or(-1), total);
    assert_eq!(payload.prompt_tokens.unwrap_or(-1), prompt);
    assert_eq!(payload.completion_tokens.unwrap_or(-1), completion);
}

#[test]
fn absent_standard_counters_read_as_zero() {
    let payload =
        run(r#"{"model": "gpt-4o"}"#, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.total_tokens, Some(0));
    assert_eq!(payload.prompt_tokens, Some(0));
    assert_eq!(payload.completion_tokens, Some(0));
}

#[test]
fn combined_usage_fills_an_empty_usage() {
    let text = r#"{"model": "gpt-4o", "combined_usage_object": {"total_tokens": 4, "prompt_tokens": 1, "completion_tokens": 3}}"#;
    let payload = run(text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(payload.total_tokens, Some(4));
    let response =
        kwargs(r#"{"usage": {"total_tokens": 1, "prompt_tokens": 1, "completion_tokens": 0}}"#);
    let used = run(text, SpendResponse::Mapping(&response), &settings()).expect("payload");
    assert_eq!(used.total_tokens, Some(1));
}

#[test]
fn ocr_usage_reads_usage_info() {
    let response = kwargs(r#"{"usage_info": {"pages_processed": 2}}"#);
    let payload = run(
        r#"{"model": "ocr-model", "call_type": "ocr"}"#,
        SpendResponse::Mapping(&response),
        &settings(),
    )
    .expect("payload");
    assert_eq!(payload.total_tokens, Some(0));
    let metadata = metadata_of(&payload);
    let additional = object(
        metadata
            .get("additional_usage_values")
            .expect("usage values"),
    );
    assert_eq!(
        additional.get("pages_processed"),
        Some(&Json::Number(Number::from(2)))
    );
}

#[rstest]
#[case::management_call("get_responses", r#"{"usage": {"total_tokens": 5}}"#, None, 0)]
#[case::cost_poll_keeps_usage(
    "get_responses",
    r#"{"usage": {"total_tokens": 5}}"#,
    Some("background_response_cost_poll"),
    5
)]
#[case::background_response_keeps_usage(
    "get_responses",
    r#"{"usage": {"total_tokens": 5}, "background": true}"#,
    None,
    5
)]
#[case::inference_keeps_usage("acompletion", r#"{"usage": {"total_tokens": 5}}"#, None, 5)]
fn unbilled_management_reads_report_no_usage(
    #[case] call_type: &str,
    #[case] response: &str,
    #[case] origin: Option<&str>,
    #[case] total: i64,
) {
    let metadata = origin.map_or(String::new(), |origin| {
        format!(r#", "metadata": {{"internal_call_origin": "{origin}"}}"#)
    });
    let text = format!(
        r#"{{"model": "m", "call_type": "{call_type}", "litellm_params": {{{metadata}}}}}"#
    );
    let message = kwargs(response);
    let payload = run(&text, SpendResponse::Mapping(&message), &settings()).expect("payload");
    assert_eq!(payload.total_tokens, Some(total));
}

#[rstest]
#[case::cached_tokens(
    r#"{"prompt_tokens_details": {"cached_tokens": 4}}"#,
    "cache_read_input_tokens",
    4
)]
#[case::creation_tokens(
    r#"{"prompt_tokens_details": {"cache_creation_tokens": 6}}"#,
    "cache_creation_input_tokens",
    6
)]
#[case::write_preferred(
    r#"{"prompt_tokens_details": {"cache_write_tokens": 3, "cache_creation_tokens": 6}}"#,
    "cache_creation_input_tokens",
    3
)]
#[case::zero_write_falls_through(
    r#"{"prompt_tokens_details": {"cache_write_tokens": 0, "cache_creation_tokens": 6}}"#,
    "cache_creation_input_tokens",
    6
)]
#[case::existing_null_blocks(r#"{"cache_read_input_tokens": null, "prompt_tokens_details": {"cached_tokens": 4}}"#, "cache_read_input_tokens", -1)]
#[case::zero_cached_is_ignored(r#"{"prompt_tokens_details": {"cached_tokens": 0}}"#, "cache_read_input_tokens", -1)]
fn cache_counts_fill_from_prompt_details(
    #[case] usage: &str,
    #[case] field: &str,
    #[case] expected: i64,
) {
    let response = format!(r#"{{"usage": {usage}}}"#);
    let message = kwargs(&response);
    let payload = run(
        r#"{"model": "m"}"#,
        SpendResponse::Mapping(&message),
        &settings(),
    )
    .expect("payload");
    let metadata = metadata_of(&payload);
    let additional = object(
        metadata
            .get("additional_usage_values")
            .expect("usage values"),
    );
    let found = match additional.get(field) {
        Some(Json::Number(number)) => number.as_i64().unwrap_or(-1),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => -1,
    };
    assert_eq!(found, expected);
}

#[test]
fn the_clean_usage_object_supplies_details_only_without_a_mapping() {
    let text = r#"{"model": "m",
        "standard_logging_object": {"metadata": {"usage_object": {"prompt_tokens_details": {"cached_tokens": 7}}}}}"#;
    let payload = run(text, SpendResponse::Absent, &settings()).expect("payload");
    let metadata = metadata_of(&payload);
    let additional = object(
        metadata
            .get("additional_usage_values")
            .expect("usage values"),
    );
    assert_eq!(
        additional.get("cache_read_input_tokens").map(number),
        Some(7)
    );
}

#[rstest]
#[case::absent_defaults_to_end(None, Some("2024-01-01T00:00:02.500000+00:00"))]
#[case::present_null_stays_null(Some(CompletionStart::Null), None)]
#[case::given_is_normalized(
    Some(CompletionStart::At(timestamp("2024-01-01T00:00:01Z"))),
    Some("2024-01-01T00:00:01+00:00")
)]
fn completion_start_times_default_to_the_end(
    #[case] given: Option<CompletionStart>,
    #[case] expected: Option<&str>,
) {
    let harness = Harness::new(r#"{"model": "m"}"#);
    let clock = Clock(0.0);
    let (_sender, tracebacks) = watch::channel(TracebackPolicy { suppress: false });
    let mut request = harness.request(SpendResponse::Absent, &clock, &tracebacks);
    request.completion_start_time = given.unwrap_or(CompletionStart::Absent);
    let payload = get_logging_payload(&request, &settings(), &Utc).expect("payload");
    assert_eq!(payload.completion_start_time.as_deref(), expected);
}

#[rstest]
#[case::trace_wins(
    r#"{"model": "m", "litellm_trace_id": "trace-1"}"#,
    None,
    Some("trace-1")
)]
#[case::defaults_to_the_injected_uuid(
    r#"{"model": "m"}"#,
    None,
    Some("00000000-0000-0000-0000-000000000001")
)]
#[case::omit_without_a_session_returns_none(
    r#"{"model": "m"}"#,
    Some(MissingSessionId::Omit),
    None
)]
#[case::omit_keeps_the_metadata_session(
    r#"{"model": "m", "litellm_params": {"metadata": {"session_id": "metadata-session"}}}"#,
    Some(MissingSessionId::Omit),
    Some("metadata-session")
)]
fn session_ids_follow_the_trace_and_the_omit_policy(
    #[case] text: &str,
    #[case] policy: Option<MissingSessionId>,
    #[case] expected: Option<&str>,
) {
    let mut current = settings();
    current.missing_session_id = policy;
    let payload = run(text, SpendResponse::Absent, &current).expect("payload");
    assert_eq!(payload.session_id.as_deref(), expected);
}

#[test]
fn batch_calls_derive_their_session_from_the_batch_id() {
    let response = kwargs(r#"{"id": "batch-1"}"#);
    let payload = run(
        r#"{"model": "m", "call_type": "acreate_batch"}"#,
        SpendResponse::Mapping(&response),
        &settings(),
    )
    .expect("payload");
    assert_eq!(payload.session_id.as_deref(), Some("batch-1"));
}

#[test]
fn a_single_group_provider_is_inferred() {
    let mut catalog = DeploymentCatalog::default();
    catalog.push(DeploymentSpec {
        model_name: "group".to_owned(),
        litellm_params: kwargs(r#"{"model": "openai/gpt-4o"}"#),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    });
    let aliases = AliasMap::new();
    let groups = RoutingGroupIndex::default();
    let inferred = |_: &str| -> Option<String> { None };
    let lookup = CatalogLookup {
        catalog: &catalog,
        model_group_alias: &aliases,
        routing_groups: &groups,
        inferred_provider: &inferred,
    };
    let fixture = Catalog::new();
    let resolution = fixture.context();
    let attribution = SpendAttributionSettings::default();
    let recognized = BTreeSet::new();
    let teams = BTreeSet::new();
    let router = SpendRouter {
        catalog: Some(&lookup),
        settings: &attribution,
        resolution: &resolution,
        recognized_groups: &recognized,
        team_public_names: &teams,
    };
    let harness = Harness::new(
        r#"{"model": "group", "litellm_params": {"metadata": {"model_group": "group"}}}"#,
    )
    .with_router(&router);
    let payload = harness
        .run(SpendResponse::Absent, &settings())
        .expect("payload");
    assert_eq!(payload.custom_llm_provider, "openai");
}

#[test]
fn oauth_and_router_provenance_are_stamped() {
    let text = r#"{"model": "claude",
        "custom_llm_provider": "anthropic",
        "litellm_params": {
            "metadata": {"used_client_oauth_token": true},
            "litellm_metadata": {"used_client_oauth_token": true}
        }}"#;
    let payload = run(text, SpendResponse::Absent, &settings()).expect("payload");
    let metadata = metadata_of(&payload);
    assert_eq!(
        metadata.get("used_client_oauth_token"),
        Some(&Json::Bool(true))
    );
    let routing = r#"{"model": "claude",
        "custom_llm_provider": "anthropic",
        "litellm_params": {"metadata": {"model_info": {"internal_router_model": true}, "model_group": "group"}}}"#;
    let routed = run(routing, SpendResponse::Absent, &settings()).expect("payload");
    let routed_metadata = metadata_of(&routed);
    let router = object(
        routed_metadata
            .get("router_metadata")
            .expect("router metadata"),
    );
    assert_eq!(
        router.get("requested_model"),
        Some(&Json::Text("group".to_owned()))
    );
    assert_eq!(
        router.get("selected_model"),
        Some(&Json::Text("claude".to_owned()))
    );
    assert_eq!(
        router.get("selected_provider"),
        Some(&Json::Text("anthropic".to_owned()))
    );
}

#[test]
fn agent_ids_keep_the_or_semantics() {
    let from_kwargs = run(
        r#"{"model": "m", "agent_id": "kwarg", "litellm_params": {"metadata": {"agent_id": "metadata"}}}"#,
        SpendResponse::Absent,
        &settings(),
    )
    .expect("payload");
    assert_eq!(from_kwargs.agent_id.as_deref(), Some("kwarg"));
    let from_metadata = run(
        r#"{"model": "m", "agent_id": "", "litellm_params": {"metadata": {"agent_id": "metadata"}}}"#,
        SpendResponse::Absent,
        &settings(),
    )
    .expect("payload");
    assert_eq!(from_metadata.agent_id.as_deref(), Some("metadata"));
    let absent = run(r#"{"model": "m"}"#, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(absent.agent_id, None);
    assert_eq!(absent.billing_agent_id, None);
}

#[test]
fn the_mcp_tool_name_comes_from_the_clean_metadata() {
    let text = r#"{"model": "m",
        "standard_logging_object": {"metadata": {"mcp_tool_call_metadata": {"namespaced_tool_name": "server__tool"}}}}"#;
    let payload = run(text, SpendResponse::Absent, &settings()).expect("payload");
    assert_eq!(
        payload.mcp_namespaced_tool_name.as_deref(),
        Some("server__tool")
    );
}

#[rstest]
#[case::cache_hit(r#"{"cache_hit": "yes"}"#, "cache_hit")]
#[case::model_info(
    r#"{"model": "m", "litellm_params": {"metadata": {"model_info": []}}}"#,
    "model_info"
)]
#[case::response_cost(r#"{"response_cost": "free"}"#, "response_cost")]
#[case::combined_usage(r#"{"combined_usage_object": []}"#, "combined_usage_object")]
#[case::standard(r#"{"standard_logging_object": []}"#, "standard_logging_object")]
fn out_of_type_fields_are_named(#[case] text: &str, #[case] field: &str) {
    let Err(error) = run(text, SpendResponse::Absent, &settings()) else {
        panic!("out of type");
    };
    assert!(error.to_string().contains(field), "{error}");
}

#[test]
fn naive_times_use_the_injected_zone() {
    let naive = NaiveDate::from_ymd_opt(2024, 1, 1)
        .expect("date")
        .and_hms_opt(0, 0, 0)
        .expect("time");
    let zone = FixedOffset::east_opt(3600).expect("zone");
    let normalized =
        super::instant(SpendTimestamp::Naive(naive), &zone, "startTime").expect("instant");
    assert_eq!(normalized, "2023-12-31T23:00:00+00:00");
}

#[test]
fn the_payload_serializes_its_times_as_isoformat_text() {
    let payload = run(r#"{"model": "m"}"#, SpendResponse::Absent, &settings()).expect("payload");
    let encoded: Json = serde_json::from_str(&serde_json::to_string(&payload).expect("encoded"))
        .expect("payload json");
    let fields = object(&encoded);
    assert_eq!(
        fields.get("startTime"),
        Some(&Json::Text("2024-01-01T00:00:00+00:00".to_owned()))
    );
    assert_eq!(
        fields.get("endTime"),
        Some(&Json::Text("2024-01-01T00:00:02.500000+00:00".to_owned()))
    );
    assert_eq!(
        fields.get("status"),
        Some(&Json::Text("success".to_owned()))
    );
    assert_eq!(fields.get("model_id"), Some(&Json::Text(String::new())));
}
