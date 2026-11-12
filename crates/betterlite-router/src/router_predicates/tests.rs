#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_error::error::ErrorClass;
use betterlite_error::rate_limit::RateLimitCategory;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::JsonMap;
use super::kinds::{deployment_tags, joins_adaptive_routing, strategy_kind};
use super::lists::{config_deployments, deployment_ids};
use super::mock_testing::{MockTestingParams, raise_mock_fallback_error, raise_mock_rate_limit};
use super::params::{
    ParamSupport, ResolvedProvider, as_output_cap, declared_param_allowlist,
    deployment_accepts_param,
};
use super::source_text::source_text;
use super::success_key::success_minute_key;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn object(text: &str) -> JsonMap {
    match json(text) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("fixture is not an object: {text}")
        }
    }
}

fn classify(model: &str) -> Option<&'static str> {
    match model {
        "auto_router/semantic" => Some("semantic"),
        "auto_router/complexity_router" => Some("complexity"),
        "auto_router/adaptive_router" => Some("adaptive"),
        "auto_router/quality_router" => Some("quality"),
        _ => None,
    }
}

fn minute_key(id: &str) -> String {
    format!("{id}:successes")
}

// LiteLLM's backslash continuation leaves the space before it plus the 20-space continuation indent
fn joined_message(label: &str, fallbacks: &str) -> String {
    format!(
        "This is a mock exception for model=gpt-4, to trigger a fallback.{}{label}={fallbacks}",
        " ".repeat(21)
    )
}

#[rstest]
#[case::semantic("auto_router/semantic", Some("semantic"))]
#[case::complexity("auto_router/complexity_router", Some("complexity"))]
#[case::adaptive("auto_router/adaptive_router", Some("adaptive"))]
#[case::quality("auto_router/quality_router", Some("quality"))]
#[case::plain("gpt-4", None)]
fn kind_router_checks_follow_the_classifier(#[case] model: &str, #[case] expected: Option<&str>) {
    let params = object(&format!(r#"{{"model": "{model}"}}"#));
    assert_eq!(strategy_kind(&params, classify), expected);
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"model": null}"#)]
#[case::non_text(r#"{"model": 7}"#)]
fn kind_router_checks_treat_an_unusable_model_as_plain(#[case] fixture: &str) {
    assert_eq!(strategy_kind(&object(fixture), classify), None);
}

#[rstest]
#[case::adaptive_router(r#"{"model": "auto_router/adaptive_router"}"#, true)]
#[case::complexity_with_adaptive(
    r#"{"model": "auto_router/complexity_router", "complexity_router_config": {"adaptive": true}}"#,
    true
)]
#[case::complexity_with_adaptive_false(
    r#"{"model": "auto_router/complexity_router", "complexity_router_config": {"adaptive": false}}"#,
    false
)]
#[case::complexity_with_empty_config(
    r#"{"model": "auto_router/complexity_router", "complexity_router_config": {}}"#,
    false
)]
#[case::complexity_without_config(r#"{"model": "auto_router/complexity_router"}"#, false)]
#[case::complexity_with_other_keys(
    r#"{"model": "auto_router/complexity_router", "complexity_router_config": {"tiers": ["fast"]}}"#,
    false
)]
#[case::semantic_with_adaptive(
    r#"{"model": "auto_router/semantic", "complexity_router_config": {"adaptive": true}}"#,
    false
)]
#[case::plain_with_adaptive(
    r#"{"model": "gpt-4", "complexity_router_config": {"adaptive": true}}"#,
    false
)]
fn kind_adaptive_join_needs_a_truthy_companion(#[case] params: &str, #[case] expected: bool) {
    assert_eq!(joins_adaptive_routing(&object(params), classify), expected);
}

#[test]
fn kind_deployment_tags_keep_their_order() {
    let deployment = object(r#"{"litellm_params": {"tags": ["zeta", "alpha"]}}"#);
    assert_eq!(
        deployment_tags(&deployment),
        vec![String::from("zeta"), String::from("alpha")]
    );
}

#[rstest]
#[case::no_litellm_params("{}")]
#[case::no_tags(r#"{"litellm_params": {}}"#)]
#[case::null_tags(r#"{"litellm_params": {"tags": null}}"#)]
#[case::empty_tags(r#"{"litellm_params": {"tags": []}}"#)]
fn kind_deployment_tags_are_empty_without_tags(#[case] deployment: &str) {
    assert!(deployment_tags(&object(deployment)).is_empty());
}

#[test]
fn kind_deployment_tags_skip_non_text_entries() {
    let deployment = object(r#"{"litellm_params": {"tags": ["kept", 7, null]}}"#);
    assert_eq!(deployment_tags(&deployment), vec![String::from("kept")]);
}

#[rstest]
#[case::integer("12", Some(12))]
#[case::zero("0", Some(0))]
#[case::negative_integer("-3", None)]
#[case::float("12.9", Some(12))]
#[case::negative_fraction("-0.5", Some(0))]
#[case::negative_float("-12.9", None)]
#[case::text_float(r#""12.9""#, Some(12))]
#[case::text_exponent(r#""1e3""#, Some(1000))]
#[case::text_positive(r#""+15""#, Some(15))]
#[case::text_spaces(r#"" 15 ""#, Some(15))]
#[case::text_negative(r#""-2""#, None)]
#[case::text_garbage(r#""abc""#, None)]
#[case::text_empty(r#""""#, None)]
#[case::text_underscore(r#""1_000""#, Some(1000))]
#[case::text_double_underscore(r#""1__0""#, None)]
#[case::text_leading_underscore(r#""_10""#, None)]
#[case::text_trailing_underscore(r#""10_""#, None)]
#[case::text_infinite(r#""inf""#, None)]
#[case::text_nan(r#""nan""#, None)]
#[case::integer_at_i64_max("9223372036854775807", Some(i64::MAX))]
#[case::integer_below_the_bound("9223372036854774784", Some(9_223_372_036_854_774_784))]
#[case::above_i64("1e19", Some(i64::MAX))]
#[case::text_above_i64(r#""1e20""#, Some(i64::MAX))]
#[case::far_above_i64("1e300", Some(i64::MAX))]
#[case::boolean_true("true", None)]
#[case::boolean_false("false", None)]
#[case::null("null", None)]
#[case::list("[]", None)]
#[case::object("{}", None)]
fn param_output_cap_coercions(#[case] value: &str, #[case] expected: Option<i64>) {
    assert_eq!(as_output_cap(&json(value)), expected);
}

#[rstest]
#[case::list(
    r#"{"allowed_openai_params": ["max_tokens", 7, "seed"]}"#,
    &["max_tokens", "seed"]
)]
#[case::missing("{}", &[])]
#[case::null(r#"{"allowed_openai_params": null}"#, &[])]
#[case::not_a_list(r#"{"allowed_openai_params": "max_tokens"}"#, &[])]
#[case::empty_list(r#"{"allowed_openai_params": []}"#, &[])]
fn param_allowlist_keeps_only_strings(#[case] params: &str, #[case] expected: &[&str]) {
    assert_eq!(
        declared_param_allowlist(&object(params)),
        expected.iter().map(|name| (*name).to_owned()).collect()
    );
}

struct Expected {
    authenticating_model: String,
    model: String,
    provider: Option<String>,
    base_model: Option<String>,
}

struct Support {
    authenticating: bool,
    resolves: bool,
    supported: Option<Vec<String>>,
    expected: Option<Expected>,
}

impl ParamSupport for Support {
    fn is_authenticating_provider(&self, model: &str, custom_llm_provider: Option<&str>) -> bool {
        if let Some(expected) = &self.expected {
            assert_eq!(model, expected.authenticating_model.as_str());
            assert_eq!(custom_llm_provider, expected.provider.as_deref());
        }
        self.authenticating
    }

    fn resolve_provider(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<ResolvedProvider> {
        if let Some(expected) = &self.expected {
            assert_eq!(model, expected.model.as_str());
            assert_eq!(custom_llm_provider, expected.provider.as_deref());
        }
        self.resolves.then(|| ResolvedProvider {
            model: model.to_owned(),
            custom_llm_provider: String::from("openai"),
        })
    }

    fn supported_openai_params(
        &self,
        _model: &str,
        custom_llm_provider: &str,
        base_model: Option<&str>,
    ) -> Option<Vec<String>> {
        assert_eq!(custom_llm_provider, "openai");
        if let Some(expected) = &self.expected {
            assert_eq!(base_model, expected.base_model.as_deref());
        }
        self.supported.clone()
    }
}

#[test]
fn param_accepts_param_without_litellm_params() {
    let deployment = object("{}");
    assert!(deployment_accepts_param(
        &deployment,
        "group",
        "seed",
        &Support {
            authenticating: false,
            resolves: true,
            supported: Some(Vec::new()),
            expected: None,
        }
    ));
}

#[test]
fn param_accepts_param_with_empty_litellm_params() {
    let deployment = object(r#"{"litellm_params": {}}"#);
    assert!(deployment_accepts_param(
        &deployment,
        "group",
        "seed",
        &Support {
            authenticating: false,
            resolves: true,
            supported: Some(Vec::new()),
            expected: None,
        }
    ));
}

#[test]
fn param_accepts_param_honors_the_allowlist() {
    let deployment =
        object(r#"{"litellm_params": {"model": "gpt-4", "allowed_openai_params": ["seed"]}}"#);
    assert!(deployment_accepts_param(
        &deployment,
        "group",
        "seed",
        &Support {
            authenticating: false,
            resolves: true,
            supported: Some(Vec::new()),
            expected: None,
        }
    ));
}

#[test]
fn param_accepts_param_honors_an_authenticating_provider() {
    let deployment = object(r#"{"litellm_params": {"model": "bedrock/anthropic.claude"}}"#);
    assert!(deployment_accepts_param(
        &deployment,
        "group",
        "seed",
        &Support {
            authenticating: true,
            resolves: true,
            supported: Some(Vec::new()),
            expected: None,
        }
    ));
}

#[test]
fn param_accepts_param_keeps_a_param_when_resolution_fails() {
    let deployment = object(r#"{"litellm_params": {"model": "unknown/model"}}"#);
    assert!(deployment_accepts_param(
        &deployment,
        "group",
        "seed",
        &Support {
            authenticating: false,
            resolves: false,
            supported: Some(Vec::new()),
            expected: None,
        }
    ));
}

#[test]
fn param_accepts_param_keeps_a_param_without_a_supported_list() {
    let deployment = object(r#"{"litellm_params": {"model": "gpt-4"}}"#);
    assert!(deployment_accepts_param(
        &deployment,
        "group",
        "seed",
        &Support {
            authenticating: false,
            resolves: true,
            supported: None,
            expected: None,
        }
    ));
}

#[test]
fn param_accepts_param_checks_the_supported_list() {
    let deployment = object(r#"{"litellm_params": {"model": "gpt-4"}}"#);
    let support = Support {
        authenticating: false,
        resolves: true,
        supported: Some(vec![String::from("max_tokens")]),
        expected: None,
    };
    assert!(deployment_accepts_param(
        &deployment,
        "group",
        "max_tokens",
        &support
    ));
    assert!(!deployment_accepts_param(
        &deployment,
        "group",
        "seed",
        &support
    ));
}

#[rstest]
#[case::model_present(
    r#"{"litellm_params": {"model": "gpt-4"}}"#,
    "gpt-4",
    "gpt-4",
    None,
    None
)]
#[case::model_empty(r#"{"litellm_params": {"model": ""}}"#, "", "group", None, None)]
#[case::model_missing(r#"{"litellm_params": {"api_key": "x"}}"#, "", "group", None, None)]
#[case::custom_provider(
    r#"{"litellm_params": {"model": "m", "custom_llm_provider": "azure"}}"#,
    "m",
    "m",
    Some("azure"),
    None
)]
#[case::empty_model_with_custom_provider(
    r#"{"litellm_params": {"model": "", "custom_llm_provider": "azure"}}"#,
    "",
    "group",
    Some("azure"),
    None
)]
#[case::model_info_first(
    r#"{"litellm_params": {"model": "m", "base_model": "params-base"}, "model_info": {"base_model": "info-base"}}"#,
    "m",
    "m",
    None,
    Some("info-base")
)]
#[case::params_fallback(
    r#"{"litellm_params": {"model": "m", "base_model": "params-base"}}"#,
    "m",
    "m",
    None,
    Some("params-base")
)]
#[case::empty_info_falls_back(
    r#"{"litellm_params": {"model": "m", "base_model": "params-base"}, "model_info": {"base_model": ""}}"#,
    "m",
    "m",
    None,
    Some("params-base")
)]
#[case::missing_base_model(r#"{"litellm_params": {"model": "m"}}"#, "m", "m", None, None)]
fn param_accepts_param_passes_the_resolved_request(
    #[case] deployment: &str,
    #[case] authenticating_model: &str,
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] base_model: Option<&str>,
) {
    let support = Support {
        authenticating: false,
        resolves: true,
        supported: Some(vec![String::from("max_tokens")]),
        expected: Some(Expected {
            authenticating_model: authenticating_model.to_owned(),
            model: model.to_owned(),
            provider: provider.map(str::to_owned),
            base_model: base_model.map(str::to_owned),
        }),
    };
    assert!(deployment_accepts_param(
        &object(deployment),
        "group",
        "max_tokens",
        &support
    ));
}

#[test]
fn list_deployment_ids_skip_null_ids() {
    let deployments = vec![
        object(r#"{"model_info": {"id": "alpha"}}"#),
        object(r#"{"model_info": {"id": null}}"#),
        object(r#"{"model_info": {}}"#),
        object("{}"),
        object(r#"{"model_info": "not-a-map"}"#),
        object(r#"{"model_info": {"id": 42}}"#),
    ];
    assert_eq!(
        deployment_ids(&deployments),
        BTreeSet::from([String::from("alpha"), String::from("42")])
    );
}

#[test]
fn list_config_deployments_skip_database_rows() {
    let model_list = vec![
        json(r#"{"model_name": "missing-info"}"#),
        json(r#"{"model_name": "null-info", "model_info": null}"#),
        json(r#"{"model_name": "text-info", "model_info": "db"}"#),
        json(r#"{"model_name": "config-flag", "model_info": {"db_model": false}}"#),
        json(r#"{"model_name": "db-row", "model_info": {"db_model": true}}"#),
        json(r#"{"model_name": "empty-info", "model_info": {}}"#),
        json(r#""not-a-row""#),
    ];
    let names: Vec<String> = config_deployments(&model_list)
        .into_iter()
        .map(|row| match row.get("model_name") {
            Some(Json::Text(name)) => name.clone(),
            _ => String::new(),
        })
        .collect();
    let expected: Vec<String> = [
        "missing-info",
        "null-info",
        "text-info",
        "config-flag",
        "empty-info",
    ]
    .iter()
    .map(|name| (*name).to_owned())
    .collect();
    assert_eq!(names, expected);
}

#[rstest]
#[case::batch_async(
    Some("aretrieve_batch"),
    r#"{"metadata": {"model_group": "gpt-4"}, "model_info": {"id": "dep"}}"#,
    None
)]
#[case::batch_sync(
    Some("retrieve_batch"),
    r#"{"metadata": {"model_group": "gpt-4"}, "model_info": {"id": "dep"}}"#,
    None
)]
#[case::no_call_type(
    None,
    r#"{"metadata": {"model_group": "gpt-4"}, "model_info": {"id": "dep"}}"#,
    Some("dep:successes")
)]
#[case::other_call_type(
    Some("acompletion"),
    r#"{"metadata": {"model_group": "gpt-4"}, "model_info": {"id": "dep"}}"#,
    Some("dep:successes")
)]
#[case::no_metadata(None, r#"{"model_info": {"id": "dep"}}"#, None)]
#[case::null_metadata(None, r#"{"metadata": null, "model_info": {"id": "dep"}}"#, None)]
#[case::metadata_not_a_map(None, r#"{"metadata": "x", "model_info": {"id": "dep"}}"#, None)]
#[case::no_model_group(None, r#"{"metadata": {}, "model_info": {"id": "dep"}}"#, None)]
#[case::null_model_group(
    None,
    r#"{"metadata": {"model_group": null}, "model_info": {"id": "dep"}}"#,
    None
)]
#[case::no_model_info(None, r#"{"metadata": {"model_group": "gpt-4"}}"#, None)]
#[case::model_info_not_a_map(
    None,
    r#"{"metadata": {"model_group": "gpt-4"}, "model_info": "x"}"#,
    None
)]
#[case::no_id(
    None,
    r#"{"metadata": {"model_group": "gpt-4"}, "model_info": {}}"#,
    None
)]
#[case::null_id(
    None,
    r#"{"metadata": {"model_group": "gpt-4"}, "model_info": {"id": null}}"#,
    None
)]
fn success_key_skips_batch_retrieves_and_missing_fields(
    #[case] call_type: Option<&str>,
    #[case] params: &str,
    #[case] expected: Option<&str>,
) {
    let key = success_minute_key(call_type, &object(params), minute_key);
    assert_eq!(key.as_deref(), expected);
}

#[test]
fn success_key_turns_an_integer_id_into_a_string() {
    let params = object(r#"{"metadata": {"model_group": "gpt-4"}, "model_info": {"id": 42}}"#);
    assert_eq!(
        success_minute_key(None, &params, minute_key),
        Some(String::from("42:successes"))
    );
}

#[rstest]
#[case::boolean_true(r#"{"mock_testing_fallbacks": true}"#, Some(true))]
#[case::boolean_false(r#"{"mock_testing_fallbacks": false}"#, Some(false))]
#[case::text_true(r#"{"mock_testing_fallbacks": "TRUE"}"#, Some(true))]
#[case::text_false(r#"{"mock_testing_fallbacks": " False "}"#, Some(false))]
#[case::text_other(r#"{"mock_testing_fallbacks": "yes"}"#, None)]
#[case::number(r#"{"mock_testing_fallbacks": 1}"#, None)]
#[case::null(r#"{"mock_testing_fallbacks": null}"#, None)]
#[case::missing("{}", None)]
fn mock_take_from_reads_one_switch(#[case] fixture: &str, #[case] expected: Option<bool>) {
    let mut request = object(fixture);
    let switches = MockTestingParams::take_from(&mut request);
    assert_eq!(switches.fallbacks, expected);
    assert!(request.is_empty());
}

#[test]
fn mock_take_from_removes_every_switch() {
    let mut request = object(
        r#"{
            "model": "gpt-4",
            "mock_testing_fallbacks": true,
            "mock_testing_context_fallbacks": "false",
            "mock_testing_content_policy_fallbacks": true
        }"#,
    );
    let switches = MockTestingParams::take_from(&mut request);
    assert_eq!(
        switches,
        MockTestingParams {
            fallbacks: Some(true),
            context_window: Some(false),
            content_policy: Some(true),
        }
    );
    assert_eq!(request.keys().collect::<Vec<_>>(), vec!["model"]);
}

#[test]
fn mock_fallback_raises_the_first_true_switch() {
    let mut request = object(
        r#"{
            "mock_testing_fallbacks": true,
            "mock_testing_context_fallbacks": true,
            "mock_testing_content_policy_fallbacks": true
        }"#,
    );
    let error =
        raise_mock_fallback_error(&mut request, Some("gpt-4"), None, None, None).unwrap_err();
    assert!(matches!(error, ErrorClass::InternalServer(_)));
    assert!(request.is_empty());
}

#[test]
fn mock_context_fallback_wins_over_content_policy() {
    let mut request = object(
        r#"{
            "mock_testing_context_fallbacks": true,
            "mock_testing_content_policy_fallbacks": true
        }"#,
    );
    let error =
        raise_mock_fallback_error(&mut request, Some("gpt-4"), None, None, None).unwrap_err();
    assert!(matches!(error, ErrorClass::ContextWindowExceeded(_)));
}

#[test]
fn mock_content_policy_fallback_is_the_last_switch() {
    let mut request = object(r#"{"mock_testing_content_policy_fallbacks": true}"#);
    let error =
        raise_mock_fallback_error(&mut request, Some("gpt-4"), None, None, None).unwrap_err();
    assert!(matches!(error, ErrorClass::ContentPolicyViolation(_)));
}

#[test]
fn mock_false_switches_raise_nothing_and_still_leave_the_request() {
    let mut request = object(
        r#"{
            "model": "gpt-4",
            "mock_testing_fallbacks": false,
            "mock_testing_context_fallbacks": false,
            "mock_testing_content_policy_fallbacks": false
        }"#,
    );
    raise_mock_fallback_error(&mut request, Some("gpt-4"), None, None, None).unwrap();
    assert_eq!(request.keys().collect::<Vec<_>>(), vec!["model"]);
}

#[test]
fn mock_fallback_accepts_a_text_switch() {
    let mut request = object(r#"{"mock_testing_fallbacks": "true"}"#);
    assert!(raise_mock_fallback_error(&mut request, None, None, None, None).is_err());
}

#[test]
fn mock_fallback_message_matches_litellm() {
    let mut request = object(r#"{"mock_testing_fallbacks": true}"#);
    let fallbacks = json(r#"[{"gpt-4": ["gpt-3.5"]}]"#);
    let error =
        raise_mock_fallback_error(&mut request, Some("gpt-4"), Some(&fallbacks), None, None)
            .unwrap_err();
    let ErrorClass::InternalServer(context) = error else {
        panic!("wrong class");
    };
    assert_eq!(
        context.message,
        "This is a mock exception for model=gpt-4, to trigger a fallback. Fallbacks=[{'gpt-4': ['gpt-3.5']}]"
    );
    assert_eq!(context.model.as_deref(), Some("gpt-4"));
    assert_eq!(context.provider.as_deref(), Some(""));
}

#[test]
fn mock_fallback_message_prints_none_for_a_missing_list() {
    let mut request = object(r#"{"mock_testing_fallbacks": true}"#);
    let error =
        raise_mock_fallback_error(&mut request, Some("gpt-4"), None, None, None).unwrap_err();
    let ErrorClass::InternalServer(context) = error else {
        panic!("wrong class");
    };
    assert_eq!(
        context.message,
        "This is a mock exception for model=gpt-4, to trigger a fallback. Fallbacks=None"
    );
}

#[test]
fn mock_fallback_message_prints_none_for_a_missing_group() {
    let mut request = object(r#"{"mock_testing_fallbacks": true}"#);
    let error = raise_mock_fallback_error(&mut request, None, None, None, None).unwrap_err();
    let ErrorClass::InternalServer(context) = error else {
        panic!("wrong class");
    };
    assert_eq!(
        context.message,
        "This is a mock exception for model=None, to trigger a fallback. Fallbacks=None"
    );
    assert_eq!(context.model, None);
}

#[test]
fn mock_context_fallback_message_matches_litellm() {
    let mut request = object(r#"{"mock_testing_context_fallbacks": true}"#);
    let fallbacks = json(r#"[{"ctx": ["backup"]}]"#);
    let error =
        raise_mock_fallback_error(&mut request, Some("gpt-4"), None, Some(&fallbacks), None)
            .unwrap_err();
    let ErrorClass::ContextWindowExceeded(details) = error else {
        panic!("wrong class");
    };
    assert_eq!(
        details.message,
        joined_message("Context_Window_Fallbacks", "[{'ctx': ['backup']}]")
    );
}

#[test]
fn mock_content_policy_fallback_message_matches_litellm() {
    let mut request = object(r#"{"mock_testing_content_policy_fallbacks": true}"#);
    let fallbacks = json(r#"[{"policy": ["deny"]}]"#);
    let error =
        raise_mock_fallback_error(&mut request, Some("gpt-4"), None, None, Some(&fallbacks))
            .unwrap_err();
    let ErrorClass::ContentPolicyViolation(details) = error else {
        panic!("wrong class");
    };
    assert_eq!(
        details.message,
        joined_message("Context_Policy_Fallbacks", "[{'policy': ['deny']}]")
    );
}

#[test]
fn mock_rate_limit_raises_with_the_only_deployment_retries() {
    let mut request = object(r#"{"model": "gpt-4", "mock_testing_rate_limit_error": true}"#);
    let deployments = vec![object(r#"{"litellm_params": {"num_retries": 3}}"#)];
    let error = raise_mock_rate_limit(&mut request, Some("gpt-4"), &deployments).unwrap_err();
    let ErrorClass::RateLimit(details) = error else {
        panic!("wrong class");
    };
    assert_eq!(
        details.context.message,
        "This is a mock exception for model=gpt-4, to trigger a rate limit error."
    );
    assert_eq!(details.context.model.as_deref(), Some("gpt-4"));
    assert_eq!(details.context.provider.as_deref(), Some(""));
    assert_eq!(details.context.num_retries, Some(3));
    assert_eq!(details.category, RateLimitCategory::VendorRateLimit);
    assert_eq!(request.keys().collect::<Vec<_>>(), vec!["model"]);
}

#[test]
fn mock_rate_limit_leaves_retries_unset_with_several_deployments() {
    let mut request = object(r#"{"mock_testing_rate_limit_error": true}"#);
    let deployments = vec![
        object(r#"{"litellm_params": {"num_retries": 3}}"#),
        object(r#"{"litellm_params": {"num_retries": 5}}"#),
    ];
    let error = raise_mock_rate_limit(&mut request, Some("gpt-4"), &deployments).unwrap_err();
    let ErrorClass::RateLimit(details) = error else {
        panic!("wrong class");
    };
    assert_eq!(details.context.num_retries, None);
}

#[test]
fn mock_rate_limit_needs_an_exact_boolean() {
    let mut request = object(r#"{"model": "gpt-4", "mock_testing_rate_limit_error": "true"}"#);
    let deployments = vec![object(r#"{"litellm_params": {"num_retries": 3}}"#)];
    raise_mock_rate_limit(&mut request, Some("gpt-4"), &deployments).unwrap();
    assert_eq!(request.keys().collect::<Vec<_>>(), vec!["model"]);
}

#[rstest]
#[case::none("null", "None")]
#[case::boolean_true("true", "True")]
#[case::boolean_false("false", "False")]
#[case::integer("42", "42")]
#[case::float("12.5", "12.5")]
#[case::float_trailing_zero("12.50", "12.5")]
#[case::exponent_number("1e3", "1000.0")]
#[case::small_float("1.5e-7", "1.5e-07")]
#[case::large_float("1e+20", "1e+20")]
#[case::text(r#""hello""#, "hello")]
#[case::list(r#"["a", true, null]"#, "['a', True, None]")]
#[case::nested(r#"[{"gpt-4": ["gpt-3.5"]}]"#, "[{'gpt-4': ['gpt-3.5']}]")]
#[case::object(r#"{"b": 1, "a": 2}"#, "{'b': 1, 'a': 2}")]
#[case::double_quote(r#"["say \"hi\""]"#, r#"['say "hi"']"#)]
#[case::single_quote(r#"["it's"]"#, r#"["it's"]"#)]
#[case::both_quotes(r#"["it's \"x\""]"#, r#"['it\'s "x"']"#)]
#[case::backslash(r#"["a\\b"]"#, r"['a\\b']")]
#[case::newline(r#"["a\nb"]"#, r"['a\nb']")]
#[case::control(r#"["a\u0001b"]"#, r"['a\x01b']")]
#[case::space(r#"["a b"]"#, "['a b']")]
#[case::delete(r#"["a\u007fb"]"#, r"['a\x7fb']")]
fn mock_messages_print_values_like_python(#[case] value: &str, #[case] expected: &str) {
    assert_eq!(source_text(&json(value)), expected);
}
