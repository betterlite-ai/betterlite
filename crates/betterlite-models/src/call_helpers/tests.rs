#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_context::request_records::RequestRecordProjection;
use betterlite_context::request_records::record::{HandleKind, InternalHandle, RequestRecord};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use super::arithmetic::{safe_divide, safe_divide_seconds};
use super::drop_params::{drop_params_env_flag, drop_params_flag, normalize_drop_params};
use super::expected_error::{ClientErrorFacts, is_expected_client_error};
use super::finish_reason::{FinishReason, map_finish_reason};
use super::headers::{
    AdditionalHeaders, HeaderSource, TrustedHeaders, process_response_headers,
    provider_response_headers, proxy_server_request_headers, set_provider_response_headers,
};
use super::metadata::{
    CallParams, MetadataBucket, MetadataMap, MetadataSlots, add_missing_spend_metadata,
    get_or_create_metadata_bucket, litellm_metadata_from_params, metadata_variable_name,
    stamped_oauth_token,
};
use super::model_name::reconstruct_model_name;
use super::stream_attributes::preserve_upstream_attributes;
use super::token_limit::coerce_token_limit;
use super::tool_calls::{remove_index_from_tool_calls, remove_items_at_indices};
use crate::decisions::Json;

fn value(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn metadata(source: &str) -> MetadataMap {
    serde_json::from_str(source).unwrap()
}

fn facts(source: &str) -> ClientErrorFacts {
    serde_json::from_str(source).unwrap()
}

fn params(source: &str) -> CallParams {
    serde_json::from_str(source).unwrap()
}

fn string_headers(source: &str) -> IndexMap<String, String> {
    serde_json::from_str(source).unwrap()
}

fn additional_headers(source: &str) -> AdditionalHeaders {
    serde_json::from_str(source).unwrap()
}

fn decimal(source: &str) -> Decimal {
    Decimal::from_str(source).unwrap()
}

const FINISH_REASON_CASES: [(&str, FinishReason); 42] = [
    ("stop_sequence", FinishReason::Stop),
    ("end_turn", FinishReason::Stop),
    ("COMPLETE", FinishReason::Stop),
    ("ERROR", FinishReason::Stop),
    ("eos_token", FinishReason::Stop),
    ("eos", FinishReason::Stop),
    ("STOP", FinishReason::Stop),
    ("FINISH_REASON_UNSPECIFIED", FinishReason::Stop),
    ("MALFORMED_FUNCTION_CALL", FinishReason::Stop),
    ("TOO_MANY_TOOL_CALLS", FinishReason::Stop),
    ("MALFORMED_RESPONSE", FinishReason::Stop),
    ("UNEXPECTED_TOOL_CALL", FinishReason::Stop),
    ("MISSING_THOUGHT_SIGNATURE", FinishReason::Stop),
    ("network_error", FinishReason::Stop),
    ("stop", FinishReason::Stop),
    ("error", FinishReason::Stop),
    ("max_tokens", FinishReason::Length),
    ("compaction", FinishReason::Length),
    ("MAX_TOKENS", FinishReason::Length),
    ("length", FinishReason::Length),
    ("tool_use", FinishReason::ToolCalls),
    ("tool_calls", FinishReason::ToolCalls),
    ("function_call", FinishReason::FunctionCall),
    ("refusal", FinishReason::ContentFilter),
    ("ERROR_TOXIC", FinishReason::ContentFilter),
    ("SAFETY", FinishReason::ContentFilter),
    ("RECITATION", FinishReason::ContentFilter),
    ("LANGUAGE", FinishReason::ContentFilter),
    ("OTHER", FinishReason::ContentFilter),
    ("BLOCKLIST", FinishReason::ContentFilter),
    ("PROHIBITED_CONTENT", FinishReason::ContentFilter),
    ("SPII", FinishReason::ContentFilter),
    ("IMAGE_SAFETY", FinishReason::ContentFilter),
    ("IMAGE_PROHIBITED_CONTENT", FinishReason::ContentFilter),
    ("NO_IMAGE", FinishReason::ContentFilter),
    ("IMAGE_RECITATION", FinishReason::ContentFilter),
    ("IMAGE_OTHER", FinishReason::ContentFilter),
    ("ESCALATION", FinishReason::ContentFilter),
    ("sensitive", FinishReason::ContentFilter),
    ("guardrail_intervened", FinishReason::ContentFilter),
    ("content_filter", FinishReason::ContentFilter),
    ("content_filtered", FinishReason::ContentFilter),
];

fn finish_reasons_normalize_to_openai_values() {
    for (raw, reason) in FINISH_REASON_CASES {
        let mapped = map_finish_reason(raw);
        assert_eq!(mapped.reason, reason, "{raw}");
        assert!(mapped.recognized, "{raw}");
    }
}

fn an_unknown_finish_reason_defaults_to_stop_without_recognizing_it() {
    let mapped = map_finish_reason("something_new");
    assert_eq!(mapped.reason, FinishReason::Stop);
    assert!(!mapped.recognized);
    assert_eq!(FinishReason::Stop.as_str(), "stop");
    assert_eq!(FinishReason::Length.as_str(), "length");
    assert_eq!(FinishReason::ToolCalls.as_str(), "tool_calls");
    assert_eq!(FinishReason::FunctionCall.as_str(), "function_call");
    assert_eq!(FinishReason::ContentFilter.as_str(), "content_filter");
}

fn token_limits_coerce_checked_integers() {
    assert_eq!(coerce_token_limit(None), None);
    assert_eq!(coerce_token_limit(Some(&value("128000"))), Some(128_000));
    assert_eq!(
        coerce_token_limit(Some(&value("\"128000\""))),
        Some(128_000)
    );
    assert_eq!(
        coerce_token_limit(Some(&value("\" 128000 \""))),
        Some(128_000)
    );
    assert_eq!(coerce_token_limit(Some(&value("\"+5\""))), Some(5));
    assert_eq!(coerce_token_limit(Some(&value("\"-5\""))), Some(-5));
    assert_eq!(coerce_token_limit(Some(&value("128000.5"))), Some(128_000));
    assert_eq!(coerce_token_limit(Some(&value("-12.75"))), Some(-12));
    assert_eq!(coerce_token_limit(Some(&value("0"))), Some(0));
    assert_eq!(coerce_token_limit(Some(&value("\"000123\""))), Some(123));
}

fn token_limits_treat_malformed_values_as_absent() {
    assert_eq!(coerce_token_limit(Some(&value("null"))), None);
    assert_eq!(coerce_token_limit(Some(&value("true"))), None);
    assert_eq!(coerce_token_limit(Some(&value("false"))), None);
    assert_eq!(coerce_token_limit(Some(&value("\"128,000\""))), None);
    assert_eq!(coerce_token_limit(Some(&value("\"\""))), None);
    assert_eq!(coerce_token_limit(Some(&value("\"128000.5\""))), None);
    assert_eq!(coerce_token_limit(Some(&value("\"1e3\""))), None);
    assert_eq!(coerce_token_limit(Some(&value("[1]"))), None);
    assert_eq!(coerce_token_limit(Some(&value("{\"a\":1}"))), None);
}

fn token_limits_outside_i64_are_absent() {
    assert_eq!(
        coerce_token_limit(Some(&value("99999999999999999999"))),
        None
    );
    assert_eq!(
        coerce_token_limit(Some(&value("\"99999999999999999999\""))),
        None
    );
    assert_eq!(coerce_token_limit(Some(&value("1e300"))), None);
}

fn expected_client_errors_use_the_code_before_the_status() {
    assert!(is_expected_client_error(Some(&facts(r#"{"code": "400"}"#))));
    assert!(is_expected_client_error(Some(&facts(
        r#"{"status_code": 401}"#
    ))));
    assert!(is_expected_client_error(Some(&facts(
        r#"{"code": null, "status_code": 401}"#
    ))));
    assert!(is_expected_client_error(Some(&facts(
        r#"{"code": "401", "status_code": 500}"#
    ))));
    assert!(is_expected_client_error(Some(&facts(
        r#"{"code": " 401 "}"#
    ))));
    assert!(is_expected_client_error(Some(&facts(r#"{"code": 404}"#))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"code": "abc", "status_code": 401}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"code": "500"}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"code": "399"}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"status_code": "500"}"#
    ))));
}

fn expected_client_errors_skip_provider_originated_failures() {
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"llm_provider": "openai", "status_code": 400}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"base_llm_exception": true, "status_code": 400}"#
    ))));
    assert!(is_expected_client_error(Some(&facts(
        r#"{"llm_provider": "", "status_code": 400}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts("{}"))));
    assert!(!is_expected_client_error(None));
}

fn expected_client_errors_keep_limiter_rejections_and_http_exceptions() {
    assert!(is_expected_client_error(Some(&facts(
        r#"{"llm_provider": "openai", "category": "litellm_rate_limit", "status_code": 429}"#
    ))));
    assert!(is_expected_client_error(Some(&facts(
        r#"{"category": "litellm_batch_rate_limit", "status_code": 429}"#
    ))));
    assert!(is_expected_client_error(Some(&facts(
        r#"{"category": "vendor_rate_limit", "status_code": 429}"#
    ))));
    assert!(is_expected_client_error(Some(&facts(
        r#"{"http_exception": true, "llm_provider": "openai", "status_code": 429}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"llm_provider": "openai", "category": "litellm_rate_limit", "status_code": 500}"#
    ))));
}

fn expected_client_errors_reject_unparsable_statuses() {
    assert!(!is_expected_client_error(Some(&facts(r#"{"code": true}"#))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"status_code": false}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"code": 404.5}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"status_code": "401.0"}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"code": {"a": 1}}"#
    ))));
    assert!(!is_expected_client_error(Some(&facts(
        r#"{"code": [400]}"#
    ))));
}

fn metadata_choice_follows_litellm_metadata_presence() {
    let absent = MetadataSlots::default();
    assert_eq!(metadata_variable_name(&absent), MetadataBucket::Metadata);
    let caller_only = MetadataSlots {
        metadata: Some(metadata(r#"{"a":1}"#)),
        litellm_metadata: None,
    };
    assert_eq!(
        metadata_variable_name(&caller_only),
        MetadataBucket::Metadata
    );
    let internal_only = MetadataSlots {
        metadata: None,
        litellm_metadata: Some(metadata(r#"{"b":2}"#)),
    };
    assert_eq!(
        metadata_variable_name(&internal_only),
        MetadataBucket::LitellmMetadata
    );
    let both = MetadataSlots {
        metadata: Some(metadata(r#"{"a":1}"#)),
        litellm_metadata: Some(metadata("{}")),
    };
    assert_eq!(
        metadata_variable_name(&both),
        MetadataBucket::LitellmMetadata
    );
}

fn get_or_create_metadata_bucket_creates_and_reuses_the_chosen_bucket() {
    let mut slots = MetadataSlots {
        metadata: Some(metadata(r#"{"a":1}"#)),
        litellm_metadata: None,
    };
    let (first_name, bucket) = get_or_create_metadata_bucket(&mut slots);
    assert_eq!(first_name, MetadataBucket::Metadata);
    assert_eq!(bucket.get("a"), Some(&value("1")));
    bucket.insert(String::from("written"), value("\"by the proxy\""));
    assert_eq!(slots.metadata.as_ref().unwrap().len(), 2);
    let (second_name, again) = get_or_create_metadata_bucket(&mut slots);
    assert_eq!(second_name, MetadataBucket::Metadata);
    assert_eq!(again.get("written"), Some(&value("\"by the proxy\"")));
    let mut empty = MetadataSlots::default();
    let (created_name, created) = get_or_create_metadata_bucket(&mut empty);
    assert_eq!(created_name, MetadataBucket::Metadata);
    assert!(created.is_empty());
    assert!(empty.metadata.is_some());
    assert_eq!(MetadataBucket::Metadata.key(), "metadata");
    assert_eq!(MetadataBucket::LitellmMetadata.key(), "litellm_metadata");
}

fn spend_metadata_copies_user_api_key_keys_and_overwrites() {
    let mut internal = metadata(r#"{"user_api_key_hash":"internal","keep":1}"#);
    let caller =
        metadata(r#"{"user_api_key_hash":"caller","user_api_key_team_id":"t","user":"u"}"#);
    add_missing_spend_metadata(&mut internal, &caller);
    assert_eq!(
        internal.get("user_api_key_hash"),
        Some(&value("\"caller\""))
    );
    assert_eq!(internal.get("user_api_key_team_id"), Some(&value("\"t\"")));
    assert_eq!(internal.get("keep"), Some(&value("1")));
    assert!(!internal.contains_key("user"));
    let mut untouched = metadata("{}");
    add_missing_spend_metadata(&mut untouched, &metadata(r#"{"xuser_api_keyx":1}"#));
    assert_eq!(untouched.get("xuser_api_keyx"), Some(&value("1")));
}

fn litellm_metadata_prefers_the_internal_bucket() {
    let both = params(
        r#"{"metadata":{"user_api_key_hash":"caller","drop":1},"litellm_metadata":{"user_api_key_alias":"alias","keep":2}}"#,
    );
    let merged = litellm_metadata_from_params(Some(&both));
    assert_eq!(merged.get("user_api_key_hash"), Some(&value("\"caller\"")));
    assert_eq!(merged.get("keep"), Some(&value("2")));
    assert!(!merged.contains_key("drop"));
    let internal_only = params(r#"{"litellm_metadata":{"user_api_key_alias":"alias"}}"#);
    assert_eq!(
        litellm_metadata_from_params(Some(&internal_only)),
        metadata(r#"{"user_api_key_alias":"alias"}"#)
    );
    let caller_only = params(r#"{"metadata":{"user":"u"}}"#);
    assert_eq!(
        litellm_metadata_from_params(Some(&caller_only)),
        metadata(r#"{"user":"u"}"#)
    );
    let empty_maps = params(r#"{"metadata":{},"litellm_metadata":{}}"#);
    assert!(litellm_metadata_from_params(Some(&empty_maps)).is_empty());
    assert!(litellm_metadata_from_params(None).is_empty());
}

fn litellm_metadata_merges_without_changing_the_internal_input() {
    let both =
        params(r#"{"metadata":{"user_api_key_hash":"caller"},"litellm_metadata":{"keep":2}}"#);
    let merged = litellm_metadata_from_params(Some(&both));
    assert_eq!(merged.get("user_api_key_hash"), Some(&value("\"caller\"")));
    let internal = both.litellm_metadata.as_ref().unwrap();
    assert!(!internal.entries.contains_key("user_api_key_hash"));
    assert_eq!(internal.entries.len(), 1);
}

fn stamped_oauth_tokens_read_the_internal_bucket_first() {
    let internal = params(
        r#"{"metadata":{"used_client_oauth_token":false},"litellm_metadata":{"used_client_oauth_token":true}}"#,
    );
    assert_eq!(
        stamped_oauth_token(internal.metadata.as_ref(), Some(&internal)),
        Some(&value("true"))
    );
    let fallback = params(r#"{"litellm_metadata":{"other":1}}"#);
    let caller = metadata(r#"{"used_client_oauth_token":false}"#);
    assert_eq!(
        stamped_oauth_token(Some(&caller), Some(&fallback)),
        Some(&value("false"))
    );
    let stamped_null = params(r#"{"litellm_metadata":{"used_client_oauth_token":null}}"#);
    assert_eq!(
        stamped_oauth_token(Some(&caller), Some(&stamped_null)),
        Some(&value("null"))
    );
    assert_eq!(stamped_oauth_token(None, Some(&fallback)), None);
    assert_eq!(stamped_oauth_token(None, None), None);
}

fn model_names_reconstruct_the_provider_prefix() {
    assert_eq!(
        reconstruct_model_name("claude-3", Some("anthropic"), Some("anthropic/claude-3")),
        "anthropic/claude-3"
    );
    assert_eq!(
        reconstruct_model_name("claude-3", Some("anthropic"), None),
        "claude-3"
    );
    assert_eq!(
        reconstruct_model_name("claude-3", Some("bedrock"), None),
        "bedrock/claude-3"
    );
    assert_eq!(
        reconstruct_model_name("anthropic/claude-3", Some("bedrock"), None),
        "anthropic/claude-3"
    );
    assert_eq!(
        reconstruct_model_name("claude-3", Some("bedrock"), Some("no-slash")),
        "bedrock/claude-3"
    );
    assert_eq!(
        reconstruct_model_name("claude-3", Some("bedrock"), Some("")),
        "bedrock/claude-3"
    );
    assert_eq!(reconstruct_model_name("", Some("bedrock"), None), "");
    assert_eq!(
        reconstruct_model_name("claude-3", Some(""), None),
        "claude-3"
    );
    assert_eq!(reconstruct_model_name("claude-3", None, None), "claude-3");
}

fn response_headers_prefix_provider_names_and_repeat_rate_limits() {
    let projected = process_response_headers(HeaderSource::Upstream(&string_headers(
        r#"{"x-ratelimit-limit-requests":"100","x-request-id":"abc","llm_provider-x-vendor":"kept"}"#,
    )));
    assert_eq!(
        projected.get("x-ratelimit-limit-requests"),
        Some(&value("\"100\""))
    );
    assert_eq!(
        projected.get("llm_provider-x-ratelimit-limit-requests"),
        Some(&value("\"100\""))
    );
    assert_eq!(
        projected.get("llm_provider-x-request-id"),
        Some(&value("\"abc\""))
    );
    assert_eq!(
        projected.get("llm_provider-x-vendor"),
        Some(&value("\"kept\""))
    );
    assert!(!projected.contains_key("llm_provider-llm_provider-x-vendor"));
}

fn response_headers_keep_trusted_internal_names_bare() {
    let trusted = TrustedHeaders {
        entries: additional_headers(
            r#"{"x-litellm-attempted-fallbacks":"1","llm_provider-x-vendor":"v","x-extra":"e"}"#,
        ),
    };
    let projected = process_response_headers(HeaderSource::Internal(&trusted));
    assert_eq!(
        projected.get("x-litellm-attempted-fallbacks"),
        Some(&value("\"1\""))
    );
    assert_eq!(
        projected.get("llm_provider-x-vendor"),
        Some(&value("\"v\""))
    );
    assert_eq!(projected.get("llm_provider-x-extra"), Some(&value("\"e\"")));
    assert!(!projected.contains_key("llm_provider-x-litellm-attempted-fallbacks"));
    let upstream = process_response_headers(HeaderSource::Upstream(&string_headers(
        r#"{"x-litellm-attempted-fallbacks":"1"}"#,
    )));
    assert!(!upstream.contains_key("x-litellm-attempted-fallbacks"));
    assert_eq!(
        upstream.get("llm_provider-x-litellm-attempted-fallbacks"),
        Some(&value("\"1\""))
    );
}

fn response_headers_drop_cookie_and_cors_names_in_both_forms() {
    let projected = process_response_headers(HeaderSource::Upstream(&string_headers(
        r#"{"set-cookie":"a=b","cookie":"c=d","access-control-allow-origin":"*","access-control-allow-credentials":"true","access-control-allow-methods":"GET","access-control-allow-headers":"x","access-control-expose-headers":"y","llm_provider-set-cookie":"a=b","llm_provider-access-control-allow-origin":"*"}"#,
    )));
    assert!(projected.is_empty(), "{projected:?}");
}

fn response_headers_keep_other_provider_headers() {
    let projected = process_response_headers(HeaderSource::Upstream(&string_headers(
        r#"{"content-type":"application/json","content-length":"2","content-security-policy":"default-src 'none'","retry-after":"3","x-vendor-trace":"t"}"#,
    )));
    assert_eq!(
        projected.get("llm_provider-content-type"),
        Some(&value("\"application/json\""))
    );
    assert_eq!(
        projected.get("llm_provider-content-length"),
        Some(&value("\"2\""))
    );
    assert_eq!(
        projected.get("llm_provider-content-security-policy"),
        Some(&value("\"default-src 'none'\""))
    );
    assert_eq!(
        projected.get("llm_provider-retry-after"),
        Some(&value("\"3\""))
    );
    assert_eq!(
        projected.get("llm_provider-x-vendor-trace"),
        Some(&value("\"t\""))
    );
    assert_eq!(projected.len(), 5);
}

fn hidden_header_projection_merges_existing_additional_headers() {
    let existing = additional_headers(
        r#"{"llm_provider-x-ratelimit-limit-requests":"existing","llm_provider-x-litellm-response-cost":0.25}"#,
    );
    let headers = string_headers(
        r#"{"x-ratelimit-limit-requests":"5","set-cookie":"a=b","x-request-id":"id"}"#,
    );
    let projection = set_provider_response_headers(&existing, &headers);
    assert_eq!(projection.headers, headers);
    assert_eq!(
        projection
            .additional_headers
            .get("llm_provider-x-ratelimit-limit-requests"),
        Some(&value("\"existing\""))
    );
    assert_eq!(
        projection
            .additional_headers
            .get("llm_provider-x-litellm-response-cost"),
        Some(&value("0.25"))
    );
    assert_eq!(
        projection
            .additional_headers
            .get("x-ratelimit-limit-requests"),
        Some(&value("\"5\""))
    );
    assert!(
        !projection
            .additional_headers
            .contains_key("llm_provider-set-cookie")
    );
}

fn hidden_header_reader_rejects_non_string_values() {
    assert_eq!(
        provider_response_headers(Some(&value(r#"{"headers":{"x-a":"1"}}"#))),
        Some(string_headers(r#"{"x-a":"1"}"#))
    );
    assert_eq!(
        provider_response_headers(Some(&value(r#"{"headers":{}}"#))),
        Some(IndexMap::new())
    );
    assert_eq!(
        provider_response_headers(Some(&value(r#"{"headers":null}"#))),
        None
    );
    assert_eq!(provider_response_headers(Some(&value("{}"))), None);
    assert_eq!(provider_response_headers(Some(&value("null"))), None);
    assert_eq!(provider_response_headers(None), None);
    assert_eq!(
        provider_response_headers(Some(&value(r#"{"headers":{"x-a":1}}"#))),
        None
    );
    assert_eq!(
        provider_response_headers(Some(&value(r#"{"headers":[]}"#))),
        None
    );
    assert_eq!(provider_response_headers(Some(&value("5"))), None);
    assert_eq!(
        provider_response_headers(Some(&value(r#"{"headers":"text"}"#))),
        None
    );
}

fn request_headers_project_the_proxy_server_request_headers() {
    let absent = None;
    assert!(proxy_server_request_headers(absent).is_empty());
    let empty = serde_json::from_str::<super::headers::RequestParams>("{}").unwrap();
    assert!(proxy_server_request_headers(Some(&empty)).is_empty());
    let with_headers: super::headers::RequestParams =
        serde_json::from_str(r#"{"proxy_server_request":{"headers":{"x-a":"1","x-b":"2"}}}"#)
            .unwrap();
    assert_eq!(
        proxy_server_request_headers(Some(&with_headers)),
        string_headers(r#"{"x-a":"1","x-b":"2"}"#)
    );
    let null_request: super::headers::RequestParams =
        serde_json::from_str(r#"{"proxy_server_request":null}"#).unwrap();
    assert!(proxy_server_request_headers(Some(&null_request)).is_empty());
    let no_headers: super::headers::RequestParams =
        serde_json::from_str(r#"{"proxy_server_request":{}}"#).unwrap();
    assert!(proxy_server_request_headers(Some(&no_headers)).is_empty());
    let null_headers: super::headers::RequestParams =
        serde_json::from_str(r#"{"proxy_server_request":{"headers":null}}"#).unwrap();
    assert!(proxy_server_request_headers(Some(&null_headers)).is_empty());
}

fn normalize_drop_params_accepts_text_flag_forms() {
    assert_eq!(normalize_drop_params(None), None);
    assert_eq!(normalize_drop_params(Some(&value("null"))), None);
    assert_eq!(normalize_drop_params(Some(&value("\"yes\""))), Some(true));
    assert_eq!(normalize_drop_params(Some(&value("\" NO \""))), Some(false));
    assert_eq!(normalize_drop_params(Some(&value("\"1\""))), Some(true));
    assert_eq!(normalize_drop_params(Some(&value("\"0\""))), Some(false));
    assert_eq!(normalize_drop_params(Some(&value("\"on\""))), Some(true));
    assert_eq!(normalize_drop_params(Some(&value("\" maybe \""))), None);
    assert_eq!(normalize_drop_params(Some(&value("\"\""))), None);
}

fn normalize_drop_params_accepts_number_and_bool_forms() {
    assert_eq!(normalize_drop_params(Some(&value("true"))), Some(true));
    assert_eq!(normalize_drop_params(Some(&value("false"))), Some(false));
    assert_eq!(normalize_drop_params(Some(&value("1"))), Some(true));
    assert_eq!(normalize_drop_params(Some(&value("0"))), Some(false));
    assert_eq!(normalize_drop_params(Some(&value("1.0"))), Some(true));
    assert_eq!(normalize_drop_params(Some(&value("0.0"))), Some(false));
    assert_eq!(normalize_drop_params(Some(&value("0.5"))), None);
    assert_eq!(normalize_drop_params(Some(&value("[]"))), None);
    assert_eq!(normalize_drop_params(Some(&value("{}"))), None);
}

fn drop_request_flags_warn_and_default_off() {
    let invalid = drop_params_flag(Some(&value("\"maybe\"")));
    assert!(!invalid.enabled);
    assert!(invalid.warned);
    let absent = drop_params_flag(None);
    assert!(!absent.enabled);
    assert!(!absent.warned);
    let null = drop_params_flag(Some(&value("null")));
    assert!(!null.enabled);
    assert!(!null.warned);
    let enabled = drop_params_flag(Some(&value("\"true\"")));
    assert!(enabled.enabled);
    assert!(!enabled.warned);
    let disabled = drop_params_flag(Some(&value("false")));
    assert!(!disabled.enabled);
    assert!(!disabled.warned);
}

fn drop_env_flags_warn_and_default_on() {
    let expected = [
        (None, false, false),
        (Some("   "), false, false),
        (Some(" yes "), true, false),
        (Some("0"), false, false),
        (Some("always"), true, true),
    ];
    for (given, enabled, warned) in expected {
        let flag = drop_params_env_flag(given);
        assert_eq!(flag.enabled, enabled, "{given:?}");
        assert_eq!(flag.warned, warned, "{given:?}");
    }
}

fn safe_divide_keeps_the_zero_denominator_default() {
    assert_eq!(
        safe_divide(decimal("10"), decimal("4"), Decimal::ZERO),
        decimal("2.5")
    );
    assert_eq!(
        safe_divide(decimal("10"), Decimal::ZERO, Decimal::ZERO),
        Decimal::ZERO
    );
    assert_eq!(
        safe_divide(decimal("10"), Decimal::ZERO, decimal("7.5")),
        decimal("7.5")
    );
    assert_eq!(
        safe_divide(decimal("-10"), decimal("4"), Decimal::ZERO),
        decimal("-2.5")
    );
    assert_eq!(
        safe_divide(decimal("0"), decimal("5"), Decimal::ZERO),
        Decimal::ZERO
    );
    assert_eq!(
        safe_divide(decimal("5"), decimal("2"), Decimal::ZERO),
        decimal("2.5")
    );
    assert_eq!(
        safe_divide(decimal("100"), Decimal::ZERO, decimal("-1")),
        decimal("-1")
    );
}

fn safe_divide_seconds_keeps_the_nonpositive_denominator_default() {
    assert_eq!(
        safe_divide_seconds(decimal("10"), decimal("4"), None),
        Some(decimal("2.5"))
    );
    assert_eq!(
        safe_divide_seconds(decimal("10"), Decimal::ZERO, None),
        None
    );
    assert_eq!(
        safe_divide_seconds(decimal("10"), Decimal::ZERO, Some(Decimal::ZERO)),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        safe_divide_seconds(decimal("10"), decimal("-2"), Some(decimal("3.5"))),
        Some(decimal("3.5"))
    );
    assert_eq!(
        safe_divide_seconds(decimal("0"), decimal("5"), None),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        safe_divide_seconds(decimal("1"), decimal("8"), None),
        Some(decimal("0.125"))
    );
}

fn tool_call_indices_are_removed() {
    let mut messages: Vec<IndexMap<String, Json>> = serde_json::from_str(
        r#"[{"role":"assistant","tool_calls":[{"id":"a","index":0,"type":"function","function":{"name":"f","arguments":"{}"}},{"id":"b","type":"function","function":{"name":"g","arguments":"{}"}}]},{"role":"user","content":"hi"},{"role":"assistant","tool_calls":null}]"#,
    )
    .unwrap();
    remove_index_from_tool_calls(&mut messages);
    let first_call = messages[0].get("tool_calls").unwrap();
    assert_eq!(
        serde_json::to_string(first_call).unwrap(),
        r#"[{"id":"a","type":"function","function":{"name":"f","arguments":"{}"}},{"id":"b","type":"function","function":{"name":"g","arguments":"{}"}}]"#
    );
    assert_eq!(messages[2].get("tool_calls"), Some(&Json::Null));
}

fn remove_items_at_indices_ignores_out_of_range_and_duplicates() {
    let mut items = vec![0_i32, 1_i32, 2_i32, 3_i32, 4_i32];
    remove_items_at_indices(&mut items, &[1, 3, 3, -1, 5, 99]);
    assert_eq!(items, vec![0_i32, 2_i32, 4_i32]);
    let mut empty: Vec<i32> = Vec::new();
    remove_items_at_indices(&mut empty, &[0, 2]);
    assert!(empty.is_empty());
    let mut untouched = vec![1_i32];
    remove_items_at_indices(&mut untouched, &[-5]);
    assert_eq!(untouched, vec![1_i32]);
}

fn record_field<'record>(
    record: &'record RequestRecord,
    bucket: &str,
    key: &str,
) -> Option<&'record RequestRecord> {
    let RequestRecord::Object(fields) = record else {
        return None;
    };
    let Some(RequestRecord::Object(bucket_fields)) = fields.get(bucket) else {
        return None;
    };
    bucket_fields.get(key)
}

fn opaque_span(identity: u64) -> RequestRecord {
    RequestRecord::Handle(InternalHandle {
        kind: HandleKind::Opaque,
        identity,
    })
}

fn span_bucket(identity: u64) -> RequestRecord {
    RequestRecord::Object(BTreeMap::from([
        ("litellm_parent_otel_span".to_owned(), opaque_span(identity)),
        ("n".to_owned(), RequestRecord::Signed(1)),
    ]))
}

fn snapshots_are_independent() {
    let mut source = RequestRecord::Object(BTreeMap::from([
        ("metadata".to_owned(), span_bucket(101)),
        ("litellm_metadata".to_owned(), span_bucket(202)),
    ]));
    let mut snapshot = RequestRecordProjection::safe_copy(&mut source, false)
        .unwrap()
        .into_owned();
    let placeholder = RequestRecord::String("placeholder".to_owned());
    for name in ["metadata", "litellm_metadata"] {
        assert_eq!(
            record_field(&snapshot, name, "litellm_parent_otel_span"),
            Some(&placeholder)
        );
        assert_eq!(
            record_field(&source, name, "litellm_parent_otel_span"),
            Some(&opaque_span(202))
        );
    }
    let RequestRecord::Object(snapshot_fields) = &mut snapshot else {
        panic!("the snapshot is not an object");
    };
    let Some(RequestRecord::Object(copied)) = snapshot_fields.get_mut("metadata") else {
        panic!("the snapshot has no metadata bucket");
    };
    copied.insert("n".to_owned(), RequestRecord::Signed(2));
    assert_eq!(
        record_field(&source, "metadata", "n"),
        Some(&RequestRecord::Signed(1))
    );
    assert_eq!(
        record_field(&snapshot, "metadata", "n"),
        Some(&RequestRecord::Signed(2))
    );
}

fn upstream_attributes_copy_only_non_openai_keys() {
    let mut response = metadata(
        r#"{"id":"chunk","created":1,"model":"groq/llama","choices":[],"citations":["a"],"kept":1}"#,
    );
    let original = metadata(
        r#"{"id":"upstream","created":2,"model":"llama","object":"chat.completion.chunk","system_fingerprint":"fp","choices":[{"index":0}],"provider_specific_fields":{"a":1},"usage":{"total_tokens":3},"x_groq":{"id":"req"},"citations":["b"],"trace":null}"#,
    );
    preserve_upstream_attributes(&mut response, &original);
    assert_eq!(
        response,
        metadata(
            r#"{"id":"chunk","created":1,"model":"groq/llama","choices":[],"citations":["b"],"kept":1,"x_groq":{"id":"req"},"trace":null}"#
        )
    );
}

fn trusted_flags_cannot_be_forged() {
    let trusted = TrustedHeaders {
        entries: additional_headers(r#"{"x-litellm-model-group":"g"}"#),
    };
    let mut trusted_projection = process_response_headers(HeaderSource::Internal(&trusted));
    trusted_projection.insert(String::from("x-litellm-added"), value("1"));
    assert_eq!(trusted.entries.len(), 1);
    let upstream = process_response_headers(HeaderSource::Upstream(&string_headers(
        r#"{"x-litellm-model-group":"spoof","x-litellm-attempted-fallbacks":"7"}"#,
    )));
    assert!(!upstream.contains_key("x-litellm-model-group"));
    assert!(!upstream.contains_key("x-litellm-attempted-fallbacks"));
    assert_eq!(
        upstream.get("llm_provider-x-litellm-attempted-fallbacks"),
        Some(&value("\"7\""))
    );
    assert_eq!(
        trusted_projection.get("x-litellm-model-group"),
        Some(&value("\"g\""))
    );
}

#[rstest]
#[case::finish_reasons_normalize_to_openai_values(finish_reasons_normalize_to_openai_values)]
#[case::an_unknown_finish_reason_defaults_to_stop_without_recognizing_it(
    an_unknown_finish_reason_defaults_to_stop_without_recognizing_it
)]
#[case::token_limits_coerce_checked_integers(token_limits_coerce_checked_integers)]
#[case::token_limits_treat_malformed_values_as_absent(
    token_limits_treat_malformed_values_as_absent
)]
#[case::token_limits_outside_i64_are_absent(token_limits_outside_i64_are_absent)]
#[case::expected_client_errors_use_the_code_before_the_status(
    expected_client_errors_use_the_code_before_the_status
)]
#[case::expected_client_errors_skip_provider_originated_failures(
    expected_client_errors_skip_provider_originated_failures
)]
#[case::expected_client_errors_keep_limiter_rejections_and_http_exceptions(
    expected_client_errors_keep_limiter_rejections_and_http_exceptions
)]
#[case::expected_client_errors_reject_unparsable_statuses(
    expected_client_errors_reject_unparsable_statuses
)]
#[case::metadata_choice_follows_litellm_metadata_presence(
    metadata_choice_follows_litellm_metadata_presence
)]
#[case::get_or_create_metadata_bucket_creates_and_reuses_the_chosen_bucket(
    get_or_create_metadata_bucket_creates_and_reuses_the_chosen_bucket
)]
#[case::spend_metadata_copies_user_api_key_keys_and_overwrites(
    spend_metadata_copies_user_api_key_keys_and_overwrites
)]
#[case::litellm_metadata_prefers_the_internal_bucket(litellm_metadata_prefers_the_internal_bucket)]
#[case::litellm_metadata_merges_without_changing_the_internal_input(
    litellm_metadata_merges_without_changing_the_internal_input
)]
#[case::stamped_oauth_tokens_read_the_internal_bucket_first(
    stamped_oauth_tokens_read_the_internal_bucket_first
)]
#[case::model_names_reconstruct_the_provider_prefix(model_names_reconstruct_the_provider_prefix)]
#[case::response_headers_prefix_provider_names_and_repeat_rate_limits(
    response_headers_prefix_provider_names_and_repeat_rate_limits
)]
#[case::response_headers_keep_trusted_internal_names_bare(
    response_headers_keep_trusted_internal_names_bare
)]
#[case::response_headers_drop_cookie_and_cors_names_in_both_forms(
    response_headers_drop_cookie_and_cors_names_in_both_forms
)]
#[case::response_headers_keep_other_provider_headers(response_headers_keep_other_provider_headers)]
#[case::hidden_header_projection_merges_existing_additional_headers(
    hidden_header_projection_merges_existing_additional_headers
)]
#[case::hidden_header_reader_rejects_non_string_values(
    hidden_header_reader_rejects_non_string_values
)]
#[case::request_headers_project_the_proxy_server_request_headers(
    request_headers_project_the_proxy_server_request_headers
)]
#[case::normalize_drop_params_accepts_text_flag_forms(
    normalize_drop_params_accepts_text_flag_forms
)]
#[case::normalize_drop_params_accepts_number_and_bool_forms(
    normalize_drop_params_accepts_number_and_bool_forms
)]
#[case::drop_request_flags_warn_and_default_off(drop_request_flags_warn_and_default_off)]
#[case::drop_env_flags_warn_and_default_on(drop_env_flags_warn_and_default_on)]
#[case::safe_divide_keeps_the_zero_denominator_default(
    safe_divide_keeps_the_zero_denominator_default
)]
#[case::safe_divide_seconds_keeps_the_nonpositive_denominator_default(
    safe_divide_seconds_keeps_the_nonpositive_denominator_default
)]
#[case::tool_call_indices_are_removed(tool_call_indices_are_removed)]
#[case::remove_items_at_indices_ignores_out_of_range_and_duplicates(
    remove_items_at_indices_ignores_out_of_range_and_duplicates
)]
#[case::trusted_flags_cannot_be_forged(trusted_flags_cannot_be_forged)]
#[case::snapshots_are_independent(snapshots_are_independent)]
#[case::upstream_attributes_copy_only_non_openai_keys(
    upstream_attributes_copy_only_non_openai_keys
)]
fn call_helpers_call_helper_tests_behavior(#[case] scenario: fn()) {
    scenario();
}
