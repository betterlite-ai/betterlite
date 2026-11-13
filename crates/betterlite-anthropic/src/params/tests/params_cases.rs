use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

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

const FORMAT: &str = r#"{"type":"json_schema","json_schema":{"schema":{"type":"object"}}}"#;

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

async fn assert_map(fixture: &Fixture, model: &str, requested: &str, expected: &str) {
    let mapped = mapped(fixture, model, requested, "{}", flags(false, false))
        .await
        .expect("maps");
    assert_eq!(Json::Object(mapped), json(expected));
}

async fn refused(fixture: &Fixture, model: &str, requested: &str) -> ParamsError {
    match mapped(fixture, model, requested, "{}", flags(false, false)).await {
        Ok(mapped) => panic!("expected an error, mapped {mapped:?}"),
        Err(error) => error,
    }
}

async fn dropped_map(
    fixture: &Fixture,
    model: &str,
    requested: &str,
    drop: DropParams,
) -> IndexMap<String, Json> {
    mapped(fixture, model, requested, "{}", drop)
        .await
        .expect("maps")
}

async fn output_configs(
    fixture: &Fixture,
    model: &str,
    global_drop: bool,
    source: &str,
) -> (Json, Json) {
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
    .await
    .expect("applies");
    let data_config = data.get("output_config").cloned().unwrap_or(Json::Null);
    let optional_config = optional.get("output_config").cloned().unwrap_or(Json::Null);
    (optional_config, data_config)
}

async fn refused_output(fixture: &Fixture, model: &str, source: &str) -> ParamsError {
    let mut session = Session::new(fixture);
    let catalogs = fixture.catalogs();
    let request = OutputConfigRequest {
        model,
        provider: "anthropic",
        global_drop: false,
    };
    let mut data = params(source);
    let mut optional = params(source);
    match apply_output_config(
        &request,
        &mut data,
        &mut optional,
        &mut session.environment(),
        &catalogs,
    )
    .await
    {
        Ok(()) => panic!("expected an error for {source}"),
        Err(error) => error,
    }
}

fn config_of(source: &str) -> Json {
    match json(source) {
        Json::Object(fields) => fields.get("output_config").cloned().unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => Json::Null,
    }
}

fn first_warning(warnings: &Warnings) -> Option<String> {
    warnings.messages().first().cloned()
}

async fn max_token_option_precedence() {
    let fixture = Fixture::new(MODELS, "{}");
    assert_map(
        &fixture,
        "plain-model",
        r#"{"max_tokens":100,"max_completion_tokens":50}"#,
        r#"{"max_tokens":50}"#,
    )
    .await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"max_completion_tokens":50,"max_tokens":100}"#,
        r#"{"max_tokens":100}"#,
    )
    .await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"max_tokens":-3}"#,
        r#"{"max_tokens":-3}"#,
    )
    .await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"max_tokens":0}"#,
        r#"{"max_tokens":0}"#,
    )
    .await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"max_tokens":true}"#,
        r#"{"max_tokens":true}"#,
    )
    .await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"max_tokens":2.5}"#,
        r#"{"max_tokens":2}"#,
    )
    .await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"max_tokens":3.5}"#,
        r#"{"max_tokens":4}"#,
    )
    .await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"max_tokens":0.4}"#,
        r#"{"max_tokens":1}"#,
    )
    .await;
    assert_map(&fixture, "plain-model", r#"{"seed":7}"#, "{}").await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000}}"#,
        r#"{"thinking":{"type":"enabled","budget_tokens":1000},"max_tokens":5096}"#,
    )
    .await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000},"max_tokens":2000}"#,
        r#"{"thinking":{"type":"enabled","budget_tokens":1000},"max_tokens":2000}"#,
    )
    .await;
    let refused = refused(&fixture, "plain-model", r#"{"max_tokens":"many"}"#).await;
    assert!(matches!(
        refused,
        ParamsError::NotNumber {
            field: "max_tokens"
        }
    ));
}

async fn reasoning_none_clears_thinking() {
    let fixture = Fixture::new(MODELS, "{}");
    let cleared = mapped(
        &fixture,
        "plain-model",
        r#"{"reasoning_effort":"none"}"#,
        r#"{"thinking":{"type":"enabled","budget_tokens":1024},"output_config":{"effort":"high"}}"#,
        flags(false, false),
    )
    .await
    .expect("maps");
    assert!(cleared.is_empty(), "{cleared:?}");
    assert_map(
        &fixture,
        "plain-model",
        r#"{"reasoning_effort":{"effort":"low"}}"#,
        r#"{"thinking":{"type":"enabled","budget_tokens":1024}}"#,
    )
    .await;
    assert_map(&fixture, "plain-model", r#"{"reasoning_effort":42}"#, "{}").await;
    assert_map(&fixture, "adaptive-model", r#"{"reasoning_effort":"minimal"}"#, r#"{"output_config":{"effort":"low"},"thinking":{"type":"adaptive","display":"summarized"}}"#).await;
    let legacy = refused(&fixture, "plain-model", r#"{"reasoning_effort":"bogus"}"#).await;
    assert!(matches!(legacy, ParamsError::Reasoning(_)));
    let adaptive = refused(
        &fixture,
        "adaptive-model",
        r#"{"reasoning_effort":"bogus"}"#,
    )
    .await;
    assert!(matches!(adaptive, ParamsError::Effort(_)));
    assert_eq!(adaptive.status_code(), Some(400));
}

async fn output_format_with_effort_drop_warning() {
    let fixture = Fixture::new(MODELS, "{}");
    let (guard, warnings) = capture_warnings();
    let (optional, data) = output_configs(
        &fixture,
        "plain-model",
        true,
        r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
    )
    .await;
    drop(guard);
    assert_eq!(optional, json(r#"{"format":{"type":"json_schema"}}"#));
    assert_eq!(data, json(r#"{"format":{"type":"json_schema"}}"#));
    assert_eq!(
        first_warning(&warnings),
        Some(DROP_OUTPUT_CONFIG_WARNING.replacen("%s", "plain-model", 1))
    );
    let (kept, _) = output_configs(
        &fixture,
        "effort-model",
        true,
        r#"{"output_config":{"effort":"high"}}"#,
    )
    .await;
    assert_eq!(kept, json(r#"{"effort":"high"}"#));
    let (format_only, _) = output_configs(
        &fixture,
        "plain-model",
        true,
        r#"{"output_config":{"format":{"type":"json_schema"}}}"#,
    )
    .await;
    assert_eq!(format_only, json(r#"{"format":{"type":"json_schema"}}"#));
    for source in [
        r#"{"output_config":null}"#,
        r#"{"output_config":{}}"#,
        r#"{"output_config":"x"}"#,
        "{}",
    ] {
        let (missing, _) = output_configs(&fixture, "plain-model", true, source).await;
        assert_eq!(missing, Json::Null, "{source}");
    }
    assert!(
        mapped(
            &fixture,
            "plain-model",
            r#"{"output_config":{"effort":"xhigh"}}"#,
            "{}",
            flags(true, false)
        )
        .await
        .expect("maps")
        .is_empty()
    );
}

async fn invalid_output_effort() {
    let fixture = Fixture::new(MODELS, "{}");
    let invalid = refused_output(
        &fixture,
        "effort-model",
        r#"{"output_config":{"effort":"bogus"}}"#,
    )
    .await;
    assert!(matches!(invalid, ParamsError::InvalidEffort { .. }));
    assert_eq!(invalid.status_code(), Some(400));
    let unsupported = refused_output(
        &fixture,
        "plain-model",
        r#"{"output_config":{"effort":"xhigh","format":{"type":"json_schema"}}}"#,
    )
    .await;
    assert!(matches!(unsupported, ParamsError::EffortUnsupported { .. }));
    assert_eq!(unsupported.status_code(), Some(400));
    for level in ["high", "medium", "low", "xhigh", "max"] {
        let source = format!(r#"{{"output_config":{{"effort":"{level}","note":"kept"}}}}"#);
        let (kept, data) = output_configs(&fixture, "effort-model", false, &source).await;
        assert_eq!(kept, config_of(&source), "{level}");
        assert_eq!(data, config_of(&source), "{level}");
    }
}

async fn speed_drop_policy() {
    let fixture = Fixture::new(MODELS, "{}");
    let refused = refused(&fixture, "plain-model", r#"{"speed":"fast"}"#).await;
    assert!(matches!(refused, ParamsError::Speed(_)));
    assert_eq!(refused.status_code(), Some(400));
    assert!(
        dropped_map(
            &fixture,
            "plain-model",
            r#"{"speed":"fast"}"#,
            flags(true, false)
        )
        .await
        .get("speed")
        .is_none()
    );
    assert!(
        dropped_map(
            &fixture,
            "plain-model",
            r#"{"speed":"fast"}"#,
            flags(false, true)
        )
        .await
        .get("speed")
        .is_none()
    );
    assert_map(
        &fixture,
        "fast-model",
        r#"{"speed":"fast"}"#,
        r#"{"speed":"fast"}"#,
    )
    .await;
    assert_map(&fixture, "plain-model", r#"{"speed":5}"#, "{}").await;
}

async fn adaptive_to_legacy_cap() {
    let fixture = Fixture::new(MODELS, "{}");
    assert_map(
        &fixture,
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":100000}"#,
        r#"{"thinking":{"type":"enabled","budget_tokens":2048}}"#,
    )
    .await;
    assert_map(
        &fixture,
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_completion_tokens":3000}"#,
        r#"{"thinking":{"type":"enabled","budget_tokens":2999}}"#,
    )
    .await;
    assert_map(
        &fixture,
        "adaptive-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":1000}"#,
        r#"{"thinking":{"type":"adaptive"}}"#,
    )
    .await;
    assert_map(&fixture, "adaptive-model", r#"{"thinking":{"type":"enabled","budget_tokens":2048}}"#, r#"{"thinking":{"type":"adaptive","display":"summarized"},"output_config":{"effort":"medium"}}"#).await;
    let (guard, warnings) = capture_warnings();
    assert_map(
        &fixture,
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":1000}"#,
        "{}",
    )
    .await;
    drop(guard);
    assert_eq!(
        first_warning(&warnings),
        Some(DROP_ADAPTIVE_THINKING_WARNING.replacen("%s", "plain-model", 1))
    );
}

async fn parameter_body_rewrites() {
    let fixture = Fixture::new(MODELS, "{}");
    let requested = r#"{"tools":[{"type":"function","function":{"name":"a","parameters":{"type":"object","properties":{}}}}],"tool_choice":"required","parallel_tool_calls":false,"stream":true,"stop":"END","temperature":0.5,"top_p":0.9,"user":"user-1","extra_headers":{"x":"1"},"cache_control":{"type":"ephemeral"},"context_management":[{"type":"compaction","compact_threshold":1000}]}"#;
    let mapped = mapped(
        &fixture,
        "plain-model",
        requested,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(
        mapped.get("tool_choice"),
        Some(&json(r#"{"type":"any","disable_parallel_tool_use":true}"#))
    );
    assert_eq!(body.get("stop_sequences"), Some(&json(r#"["END"]"#)));
    assert_eq!(
        mapped.get("metadata"),
        Some(&json(r#"{"user_id":"user-1"}"#))
    );
    assert_eq!(
        mapped.get("cache_control"),
        Some(&json(r#"{"type":"ephemeral"}"#))
    );
    assert!(matches!(body.get("tools"), Some(Json::List(items)) if items.len() == 1));
}

async fn structured_format_selection() {
    let fixture = Fixture::new(MODELS, "{}");
    let native = format!(r#"{{"response_format":{FORMAT}}}"#);
    let native_map = mapped(&fixture, "native-model", &native, "{}", flags(false, false))
        .await
        .expect("maps");
    assert_eq!(native_map.get("json_mode"), Some(&Json::Bool(true)));
    assert!(matches!(
        native_map.get("output_format"),
        Some(Json::Object(_))
    ));
    let fallback = mapped(
        &fixture,
        "fallback-model",
        &native,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert_eq!(
        fallback.get("tool_choice"),
        Some(&json(r#"{"name":"json_tool_call","type":"tool"}"#))
    );
    assert_eq!(fallback.get("json_mode"), Some(&Json::Bool(true)));
    let thinking = mapped(
        &fixture,
        "fallback-model",
        &format!(r#"{{"reasoning_effort":"low","response_format":{FORMAT}}}"#),
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert!(thinking.get("tool_choice").is_none());
    let no_forced = mapped(
        &fixture,
        "no-forced-tools",
        &native,
        "{}",
        flags(false, false),
    )
    .await
    .expect("maps");
    assert!(no_forced.get("tool_choice").is_none());
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
