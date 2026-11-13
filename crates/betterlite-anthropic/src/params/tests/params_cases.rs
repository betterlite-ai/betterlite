use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::fixture::{Fixture, Session, Warnings, capture_warnings};
use crate::anthropic_capability_catalog::DropParams;
use crate::capabilities::speed::DROP_UNSUPPORTED_SPEED_WARNING;
use crate::params::error::ParamsError;
use crate::params::output_config::apply as apply_output_config;
use crate::params::output_config::{DROP_OUTPUT_CONFIG_WARNING, OutputConfigRequest};
use crate::params::thinking::DROP_ADAPTIVE_THINKING_WARNING;
use crate::params::{DEFAULT_MAX_TOKENS, ParamsRequest, map_openai_params};

const MODELS: &str = r#"{
    "native-model": {"litellm_provider": "anthropic", "supports_native_structured_output": true},
    "fallback-model": {"litellm_provider": "anthropic"},
    "adaptive-model": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true},
    "effort-model": {"litellm_provider": "anthropic", "supports_output_config": true, "supports_max_reasoning_effort": true},
    "plain-model": {"litellm_provider": "anthropic"},
    "fast-model": {"litellm_provider": "anthropic", "supports_speed": true},
    "no-forced-tools": {"litellm_provider": "anthropic", "supports_forced_tool_use": false}
}"#;

const RESPONSE_FORMAT: &str =
    r#"{"type":"json_schema","json_schema":{"schema":{"type":"object"}}}"#;

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("case JSON")
}

fn params(source: &str) -> IndexMap<String, Json> {
    match json(source) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            IndexMap::new()
        }
    }
}

const fn flags(call: bool, global: bool) -> DropParams {
    DropParams { global, call }
}

async fn mapped(
    fixture: &Fixture,
    model: &str,
    non_default: &str,
    initial: &str,
    drop: DropParams,
) -> Result<IndexMap<String, Json>, ParamsError> {
    let mut session = Session::new(fixture);
    let catalogs = fixture.catalogs();
    let budgets = Settings::default();
    let request = ParamsRequest {
        model,
        provider: "anthropic",
        drop,
        budgets: &budgets,
        default_max_tokens: DEFAULT_MAX_TOKENS,
        catalogs: &catalogs,
    };
    let requested = params(non_default);
    let mut optional = params(initial);
    map_openai_params(
        &request,
        &requested,
        &mut optional,
        &mut session.environment(),
    )
    .await?;
    Ok(optional)
}

async fn output_mapped(
    fixture: &Fixture,
    model: &str,
    global_drop: bool,
    source: &str,
) -> Result<(IndexMap<String, Json>, IndexMap<String, Json>), ParamsError> {
    let mut session = Session::new(fixture);
    let catalogs = fixture.catalogs();
    let request = OutputConfigRequest {
        model,
        provider: "anthropic",
        global_drop,
    };
    let mut data = params(source);
    let mut optional = params(source);
    apply_output_config(
        &request,
        &mut data,
        &mut optional,
        &mut session.environment(),
        &catalogs,
    )
    .await?;
    Ok((data, optional))
}

fn get<'map>(map: &'map IndexMap<String, Json>, key: &str) -> Option<&'map Json> {
    map.get(key)
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn first_warning(warnings: &Warnings) -> Option<String> {
    warnings.messages().first().cloned()
}

fn output_config_of(map: &IndexMap<String, Json>) -> Option<&Json> {
    map.get("output_config")
}

fn output_config_in(source: &str) -> Json {
    match json(source) {
        Json::Object(fields) => fields.get("output_config").cloned().unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Json::Null,
    }
}

async fn max_tokens_follows_request_order(fixture: &Fixture) {
    let first = mapped(
        fixture,
        "plain-model",
        r#"{"max_tokens":100,"max_completion_tokens":50}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&first, "max_tokens"), Some(&number(50)));
    let second = mapped(
        fixture,
        "plain-model",
        r#"{"max_completion_tokens":50,"max_tokens":100}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&second, "max_tokens"), Some(&number(100)));
}

async fn ints_pass_and_floats_round(fixture: &Fixture) {
    for (given, expected) in [
        ("-3", number(-3)),
        ("0", number(0)),
        ("true", Json::Bool(true)),
        ("2.5", number(2)),
        ("3.5", number(4)),
        ("0.4", number(1)),
    ] {
        let source = format!(r#"{{"max_tokens":{given}}}"#);
        let result = mapped(fixture, "plain-model", &source, "{}", flags(false, false))
            .await
            .expect("maps");
        assert_eq!(get(&result, "max_tokens"), Some(&expected), "{given}");
    }
}

async fn unknown_options_stay_unmapped(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"seed":7,"logprobs":true}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert!(result.is_empty());
}

async fn a_refused_max_tokens_names_the_field(fixture: &Fixture) {
    let refused = mapped(
        fixture,
        "plain-model",
        r#"{"max_tokens":"many"}"#,
        "{}",
        flags(false, false),
    )
    .await;
    assert!(matches!(
        refused,
        Err(ParamsError::NotNumber {
            field: "max_tokens"
        })
    ));
}

async fn thinking_without_max_tokens_adds_the_budget(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000}}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(
        get(&result, "max_tokens"),
        Some(&number(1000 + DEFAULT_MAX_TOKENS))
    );
    let kept = mapped(
        fixture,
        "plain-model",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000},"max_tokens":2000}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&kept, "max_tokens"), Some(&number(2000)));
}

async fn max_token_option_precedence() {
    let fixture = Fixture::new(MODELS, "{}");
    max_tokens_follows_request_order(&fixture).await;
    ints_pass_and_floats_round(&fixture).await;
    unknown_options_stay_unmapped(&fixture).await;
    a_refused_max_tokens_names_the_field(&fixture).await;
    thinking_without_max_tokens_adds_the_budget(&fixture).await;
}

async fn none_clears_thinking_and_output_config(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"reasoning_effort":"none"}"#,
        r#"{"thinking":{"type":"enabled","budget_tokens":1024},"output_config":{"effort":"high"}}"#,
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&result, "thinking"), None);
    assert_eq!(output_config_of(&result), None);
}

async fn a_dict_effort_is_read_and_nonstrings_ignored(fixture: &Fixture) {
    let dict = mapped(
        fixture,
        "plain-model",
        r#"{"reasoning_effort":{"effort":"low"}}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(
        get(&dict, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":1024}"#))
    );
    let number_effort = mapped(
        fixture,
        "plain-model",
        r#"{"reasoning_effort":42}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&number_effort, "thinking"), None);
}

async fn adaptive_effort_replaces_output_config(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "adaptive-model",
        r#"{"reasoning_effort":"minimal"}"#,
        r#"{"output_config":{"format":{"type":"json_schema"}}}"#,
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(
        output_config_of(&result),
        Some(&json(r#"{"effort":"low"}"#))
    );
    assert_eq!(
        get(&result, "thinking"),
        Some(&json(r#"{"type":"adaptive","display":"summarized"}"#))
    );
}

async fn an_invalid_effort_is_a_bad_request(fixture: &Fixture) {
    let legacy = mapped(
        fixture,
        "plain-model",
        r#"{"reasoning_effort":"bogus"}"#,
        "{}",
        flags(false, false),
    )
    .await;
    assert!(matches!(legacy, Err(ParamsError::Reasoning(_))));
    let adaptive = mapped(
        fixture,
        "adaptive-model",
        r#"{"reasoning_effort":"bogus"}"#,
        "{}",
        flags(false, false),
    )
    .await;
    assert!(matches!(adaptive, Err(ParamsError::Effort(_))));
    assert_eq!(adaptive.expect_err("raises").status_code(), Some(400));
}

async fn reasoning_none_clears_thinking() {
    let fixture = Fixture::new(MODELS, "{}");
    none_clears_thinking_and_output_config(&fixture).await;
    a_dict_effort_is_read_and_nonstrings_ignored(&fixture).await;
    adaptive_effort_replaces_output_config(&fixture).await;
    an_invalid_effort_is_a_bad_request(&fixture).await;
}

async fn the_drop_branch_keeps_only_the_format(fixture: &Fixture, warnings: &Warnings) {
    let (data, optional) = output_mapped(
        fixture,
        "plain-model",
        true,
        r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
    )
    .await
    .expect("applies");
    assert_eq!(
        output_config_of(&optional),
        Some(&json(r#"{"format":{"type":"json_schema"}}"#))
    );
    assert_eq!(
        output_config_of(&data),
        Some(&json(r#"{"format":{"type":"json_schema"}}"#))
    );
    assert_eq!(
        first_warning(warnings),
        Some(DROP_OUTPUT_CONFIG_WARNING.replacen("%s", "plain-model", 1))
    );
}

async fn a_format_only_config_is_kept(fixture: &Fixture) {
    let (data, optional) = output_mapped(
        fixture,
        "plain-model",
        true,
        r#"{"output_config":{"format":{"type":"json_schema"}}}"#,
    )
    .await
    .expect("applies");
    assert_eq!(
        output_config_of(&optional),
        Some(&json(r#"{"format":{"type":"json_schema"}}"#))
    );
    assert_eq!(
        output_config_of(&data),
        Some(&json(r#"{"format":{"type":"json_schema"}}"#))
    );
}

async fn a_supported_model_keeps_the_whole_config(fixture: &Fixture) {
    let source = r#"{"output_config":{"effort":"max","format":{"type":"json_schema"}}}"#;
    let (data, optional) = output_mapped(fixture, "effort-model", true, source)
        .await
        .expect("applies");
    let expected = output_config_in(source);
    assert_eq!(output_config_of(&optional), Some(&expected));
    assert_eq!(output_config_of(&data), Some(&expected));
}

async fn falsey_and_non_dict_configs_are_ignored(fixture: &Fixture) {
    for source in [
        r#"{"output_config":null}"#,
        r#"{"output_config":{}}"#,
        r#"{"output_config":"effort"}"#,
        r#"{"output_config":[]}"#,
        "{}",
    ] {
        let (data, optional) = output_mapped(fixture, "plain-model", true, source)
            .await
            .expect("applies");
        assert_eq!(output_config_of(&optional), None, "{source}");
        assert_eq!(output_config_of(&data), None, "{source}");
    }
}

async fn a_per_call_drop_does_not_take_the_branch(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"output_config":{"effort":"xhigh"}}"#,
        "{}",
        flags(true, false),
    )
    .await;
    assert!(
        result.is_empty(),
        "output_config is not an OpenAI parameter"
    );
}

async fn output_format_with_effort_drop_warning() {
    let fixture = Fixture::new(MODELS, "{}");
    let (guard, warnings) = capture_warnings();
    the_drop_branch_keeps_only_the_format(&fixture, &warnings).await;
    drop(guard);
    a_format_only_config_is_kept(&fixture).await;
    a_supported_model_keeps_the_whole_config(&fixture).await;
    falsey_and_non_dict_configs_are_ignored(&fixture).await;
    a_per_call_drop_does_not_take_the_branch(&fixture).await;
}

async fn an_invalid_output_effort_is_a_bad_request(fixture: &Fixture) {
    let invalid = output_mapped(
        fixture,
        "effort-model",
        true,
        r#"{"output_config":{"effort":"bogus"}}"#,
    )
    .await;
    assert!(matches!(invalid, Err(ParamsError::InvalidEffort { .. })));
    assert_eq!(invalid.expect_err("raises").status_code(), Some(400));
}

async fn an_unsupported_level_is_a_bad_request(fixture: &Fixture) {
    let xhigh = output_mapped(
        fixture,
        "plain-model",
        false,
        r#"{"output_config":{"effort":"xhigh","format":{"type":"json_schema"}}}"#,
    )
    .await;
    assert!(matches!(xhigh, Err(ParamsError::EffortUnsupported { .. })));
    assert_eq!(xhigh.expect_err("raises").status_code(), Some(400));
}

async fn the_valid_levels_pass_and_keep_extra_keys(fixture: &Fixture) {
    for level in ["high", "medium", "low", "xhigh", "max"] {
        let source = format!(r#"{{"output_config":{{"effort":"{level}","note":"kept"}}}}"#);
        let (_, optional) = output_mapped(fixture, "effort-model", false, &source)
            .await
            .expect("applies");
        let expected = output_config_in(&source);
        assert_eq!(output_config_of(&optional), Some(&expected), "{level}");
    }
}

async fn a_format_only_mapping_needs_no_effort_support(fixture: &Fixture) {
    let (data, optional) = output_mapped(
        fixture,
        "plain-model",
        false,
        r#"{"output_config":{"format":{"type":"json_schema"}}}"#,
    )
    .await
    .expect("applies");
    assert_eq!(
        output_config_of(&optional),
        Some(&json(r#"{"format":{"type":"json_schema"}}"#))
    );
    assert_eq!(
        output_config_of(&data),
        Some(&json(r#"{"format":{"type":"json_schema"}}"#))
    );
}

async fn invalid_output_effort() {
    let fixture = Fixture::new(MODELS, "{}");
    an_invalid_output_effort_is_a_bad_request(&fixture).await;
    an_unsupported_level_is_a_bad_request(&fixture).await;
    the_valid_levels_pass_and_keep_extra_keys(&fixture).await;
    a_format_only_mapping_needs_no_effort_support(&fixture).await;
}

async fn speed_is_refused_until_a_drop_is_set(fixture: &Fixture) {
    let refused = mapped(
        fixture,
        "plain-model",
        r#"{"speed":"fast"}"#,
        "{}",
        flags(false, false),
    )
    .await;
    assert!(matches!(refused, Err(ParamsError::Speed(_))));
    assert_eq!(refused.expect_err("raises").status_code(), Some(400));
    let call_drop = mapped(
        fixture,
        "plain-model",
        r#"{"speed":"fast"}"#,
        "{}",
        flags(true, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&call_drop, "speed"), None);
    let kept = mapped(
        fixture,
        "fast-model",
        r#"{"speed":"fast"}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&kept, "speed"), Some(&json(r#""fast""#)));
}

async fn a_nonstring_speed_is_left_alone(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"speed":5}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&result, "speed"), None);
}

async fn a_global_drop_warns_once(fixture: &Fixture, warnings: &Warnings) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"speed":"fast"}"#,
        "{}",
        flags(false, true),
    )
    .await
    .expect("maps");
    assert_eq!(get(&result, "speed"), None);
    assert_eq!(
        first_warning(warnings),
        Some(DROP_UNSUPPORTED_SPEED_WARNING.replacen("%s", "plain-model", 1))
    );
}

async fn speed_drop_policy() {
    let fixture = Fixture::new(MODELS, "{}");
    speed_is_refused_until_a_drop_is_set(&fixture).await;
    a_nonstring_speed_is_left_alone(&fixture).await;
    let (guard, warnings) = capture_warnings();
    a_global_drop_warns_once(&fixture, &warnings).await;
    drop(guard);
}

async fn adaptive_thinking_uses_the_medium_budget(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":100000}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(
        get(&result, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":2048}"#))
    );
}

async fn a_mid_cap_bounds_the_budget(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_completion_tokens":3000}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(
        get(&result, "thinking"),
        Some(&json(r#"{"type":"enabled","budget_tokens":2999}"#))
    );
}

async fn an_adaptive_model_keeps_the_explicit_type(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "adaptive-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":1000}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(
        get(&result, "thinking"),
        Some(&json(r#"{"type":"adaptive"}"#))
    );
}

async fn a_small_cap_drops_adaptive_thinking(fixture: &Fixture, warnings: &Warnings) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":1000}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&result, "thinking"), None);
    assert_eq!(
        first_warning(warnings),
        Some(DROP_ADAPTIVE_THINKING_WARNING.replacen("%s", "plain-model", 1))
    );
}

async fn adaptive_to_legacy_cap() {
    let fixture = Fixture::new(MODELS, "{}");
    adaptive_thinking_uses_the_medium_budget(&fixture).await;
    a_mid_cap_bounds_the_budget(&fixture).await;
    an_adaptive_model_keeps_the_explicit_type(&fixture).await;
    let (guard, warnings) = capture_warnings();
    a_small_cap_drops_adaptive_thinking(&fixture, &warnings).await;
    drop(guard);
}

async fn every_rewrite_lands_in_the_body(fixture: &Fixture) {
    let requested = r#"{"tools":[{"type":"function","function":{"name":"a","parameters":{"type":"object","properties":{}}}}],"tool_choice":"required","parallel_tool_calls":false,"stream":true,"stop":"END","temperature":0.5,"top_p":0.9,"user":"user-1","extra_headers":{"x":"1"},"cache_control":{"type":"ephemeral"},"context_management":[{"type":"compaction","compact_threshold":1000}]}"#;
    let result = mapped(fixture, "plain-model", requested, "{}", flags(false, false))
        .await
        .expect("maps");
    assert_eq!(
        get(&result, "tool_choice"),
        Some(&json(r#"{"type":"any","disable_parallel_tool_use":true}"#))
    );
    assert_eq!(get(&result, "stop_sequences"), Some(&json(r#"["END"]"#)));
    assert_eq!(get(&result, "temperature"), Some(&json("0.5")));
    assert_eq!(get(&result, "top_p"), Some(&json("0.9")));
    assert_eq!(
        get(&result, "metadata"),
        Some(&json(r#"{"user_id":"user-1"}"#))
    );
    assert_eq!(get(&result, "extra_headers"), Some(&json(r#"{"x":"1"}"#)));
    assert_eq!(
        get(&result, "cache_control"),
        Some(&json(r#"{"type":"ephemeral"}"#))
    );
    assert!(matches!(get(&result, "tools"), Some(Json::List(items)) if items.len() == 1));
}

async fn weak_values_are_not_copied(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"stream":false,"user":"someone@example.com","extra_headers":null,"cache_control":"ephemeral"}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&result, "stream"), None);
    assert_eq!(get(&result, "metadata"), None);
    assert_eq!(get(&result, "extra_headers"), Some(&Json::Null));
    assert_eq!(get(&result, "cache_control"), None);
}

async fn web_search_appends_one_tool(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "plain-model",
        r#"{"web_search_options":{"search_context_size":"low"}}"#,
        r#"{"tools":[{"name":"a","input_schema":{"type":"object"}}]}"#,
        flags(false, false),
    )
    .await
    .expect("maps");
    assert!(
        matches!(get(&result, "tools"), Some(Json::List(items)) if items.len() == 2),
        "the hosted tool is appended"
    );
}

async fn parameter_body_rewrites() {
    let fixture = Fixture::new(MODELS, "{}");
    every_rewrite_lands_in_the_body(&fixture).await;
    weak_values_are_not_copied(&fixture).await;
    web_search_appends_one_tool(&fixture).await;
}

async fn a_native_model_emits_output_format(fixture: &Fixture) {
    let requested = format!(r#"{{"response_format":{RESPONSE_FORMAT}}}"#);
    let result = mapped(
        fixture,
        "native-model",
        &requested,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&result, "json_mode"), Some(&Json::Bool(true)));
    assert!(matches!(
        get(&result, "output_format"),
        Some(Json::Object(_))
    ));
}

async fn a_plain_model_gets_the_tool_fallback(fixture: &Fixture) {
    let requested = format!(r#"{{"response_format":{RESPONSE_FORMAT}}}"#);
    let result = mapped(
        fixture,
        "fallback-model",
        &requested,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(
        get(&result, "tool_choice"),
        Some(&json(r#"{"name":"json_tool_call","type":"tool"}"#))
    );
    assert!(matches!(get(&result, "tools"), Some(Json::List(items)) if items.len() == 1));
    assert_eq!(get(&result, "json_mode"), Some(&Json::Bool(true)));
}

async fn thinking_and_forced_tools_change_the_choice(fixture: &Fixture) {
    let thinking = format!(r#"{{"reasoning_effort":"low","response_format":{RESPONSE_FORMAT}}}"#);
    let with_thinking = mapped(
        fixture,
        "fallback-model",
        &thinking,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&with_thinking, "tool_choice"), None);
    let no_forced = format!(r#"{{"response_format":{RESPONSE_FORMAT}}}"#);
    let forced = mapped(
        fixture,
        "no-forced-tools",
        &no_forced,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&forced, "tool_choice"), None);
}

async fn a_text_format_maps_nothing(fixture: &Fixture) {
    let result = mapped(
        fixture,
        "fallback-model",
        r#"{"response_format":{"type":"text"}}"#,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(get(&result, "json_mode"), None);
    assert_eq!(get(&result, "tools"), None);
}

async fn structured_format_selection() {
    let fixture = Fixture::new(MODELS, "{}");
    a_native_model_emits_output_format(&fixture).await;
    a_plain_model_gets_the_tool_fallback(&fixture).await;
    thinking_and_forced_tools_change_the_choice(&fixture).await;
    a_text_format_maps_nothing(&fixture).await;
}

#[derive(Clone, Copy)]
enum Scenario {
    MaxTokenOptionPrecedence,
    ReasoningNoneClearsThinking,
    OutputFormatWithEffortDropWarning,
    InvalidOutputEffort,
    SpeedDropPolicy,
    AdaptiveToLegacyCap,
    ParameterBodyRewrites,
    StructuredFormatSelection,
}

#[rstest]
#[case::case_1_max_token_option_precedence(Scenario::MaxTokenOptionPrecedence)]
#[case::case_2_reasoning_none_clears_thinking(Scenario::ReasoningNoneClearsThinking)]
#[case::case_3_output_format_with_effort_drop_warning(Scenario::OutputFormatWithEffortDropWarning)]
#[case::case_4_invalid_output_effort(Scenario::InvalidOutputEffort)]
#[case::case_5_speed_drop_policy(Scenario::SpeedDropPolicy)]
#[case::case_6_adaptive_to_legacy_cap(Scenario::AdaptiveToLegacyCap)]
#[case::case_7_parameter_body_rewrites(Scenario::ParameterBodyRewrites)]
#[case::case_8_structured_format_selection(Scenario::StructuredFormatSelection)]
#[tokio::test]
async fn params_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::MaxTokenOptionPrecedence => max_token_option_precedence().await,
        Scenario::ReasoningNoneClearsThinking => reasoning_none_clears_thinking().await,
        Scenario::OutputFormatWithEffortDropWarning => {
            output_format_with_effort_drop_warning().await
        }
        Scenario::InvalidOutputEffort => invalid_output_effort().await,
        Scenario::SpeedDropPolicy => speed_drop_policy().await,
        Scenario::AdaptiveToLegacyCap => adaptive_to_legacy_cap().await,
        Scenario::ParameterBodyRewrites => parameter_body_rewrites().await,
        Scenario::StructuredFormatSelection => structured_format_selection().await,
    }
}
