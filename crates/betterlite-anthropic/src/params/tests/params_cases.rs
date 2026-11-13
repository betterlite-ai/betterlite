use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::fixture::{Fixture, Session, capture_warnings};
use crate::anthropic_capability_catalog::DropParams;
use crate::capabilities::speed::DROP_UNSUPPORTED_SPEED_WARNING;
use crate::params::error::ParamsError;
use crate::params::output_config::apply as apply_output_config;
use crate::params::output_config::{DROP_OUTPUT_CONFIG_WARNING, OutputConfigRequest};
use crate::params::thinking::DROP_ADAPTIVE_THINKING_WARNING;
use crate::params::{DEFAULT_MAX_TOKENS, ParamsRequest, map_openai_params};

const MODELS: &str = r#"{
    "native-model": {"litellm_provider": "anthropic", "supports_native_structured_output": true},
    "adaptive-model": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true},
    "effort-model": {"litellm_provider": "anthropic", "supports_output_config": true, "supports_max_reasoning_effort": true, "supports_xhigh_reasoning_effort": true},
    "plain-model": {"litellm_provider": "anthropic"},
    "fast-model": {"litellm_provider": "anthropic", "supports_speed": true},
    "no-forced-tools": {"litellm_provider": "anthropic", "supports_forced_tool_use": false}
}"#;

const FORMAT: &str = r#"{"type":"json_schema","json_schema":{"schema":{"type":"object"}}}"#;
const FORMAT_ONLY: &str = r#"{"format":{"type":"json_schema"}}"#;
const EFFORT_AND_FORMAT: &str = r#"{"effort":"high","format":{"type":"json_schema"}}"#;
const NONE: DropParams = DropParams {
    global: false,
    call: false,
};
const GLOBAL: DropParams = DropParams {
    global: true,
    call: false,
};
const PER_CALL: DropParams = DropParams {
    global: false,
    call: true,
};

// model, request, optional params before the call, optional params after it
type MapCase = (&'static str, &'static str, &'static str, &'static str);

// model, global drop, output_config sent, optional params after, body after, warns
type OutputCase = (
    &'static str,
    bool,
    &'static str,
    &'static str,
    &'static str,
    bool,
);

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

async fn try_map(
    model: &str,
    requested: &str,
    initial: &str,
    drop: DropParams,
) -> Result<IndexMap<String, Json>, ParamsError> {
    let fixture = Fixture::new(MODELS, "{}");
    let mut session = Session::new(&fixture);
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
    let mut optional = params(initial);
    map_openai_params(
        &request,
        &params(requested),
        &mut optional,
        &mut session.environment(),
    )
    .await?;
    Ok(optional)
}

async fn map_with(model: &str, requested: &str, drop: DropParams) -> IndexMap<String, Json> {
    try_map(model, requested, "{}", drop)
        .await
        .expect("the request maps")
}

async fn refused(model: &str, requested: &str) -> ParamsError {
    match try_map(model, requested, "{}", NONE).await {
        Ok(mapped) => panic!("expected an error, mapped {mapped:?}"),
        Err(error) => error,
    }
}

async fn check_maps(cases: &[MapCase]) {
    for (model, requested, initial, expected) in cases {
        let mapped = try_map(model, requested, initial, NONE)
            .await
            .expect("the request maps");
        assert_eq!(
            Json::Object(mapped),
            json(expected),
            "{model} {requested} {initial}"
        );
    }
}

// the optional params and the body after _apply_output_config, a missing config reading null
async fn output_run(
    model: &str,
    global_drop: bool,
    source: &str,
) -> Result<(Json, Json), ParamsError> {
    let fixture = Fixture::new(MODELS, "{}");
    let mut session = Session::new(&fixture);
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
    let read = |fields: &IndexMap<String, Json>| {
        fields.get("output_config").cloned().unwrap_or(Json::Null)
    };
    Ok((read(&optional), read(&data)))
}

const MAX_TOKEN_CASES: [MapCase; 13] = [
    (
        "plain-model",
        r#"{"max_tokens":100,"max_completion_tokens":50}"#,
        "{}",
        r#"{"max_tokens":50}"#,
    ),
    (
        "plain-model",
        r#"{"max_completion_tokens":50,"max_tokens":100}"#,
        "{}",
        r#"{"max_tokens":100}"#,
    ),
    (
        "plain-model",
        r#"{"max_tokens":-3}"#,
        "{}",
        r#"{"max_tokens":-3}"#,
    ),
    (
        "plain-model",
        r#"{"max_tokens":0}"#,
        "{}",
        r#"{"max_tokens":0}"#,
    ),
    (
        "plain-model",
        r#"{"max_tokens":true}"#,
        "{}",
        r#"{"max_tokens":true}"#,
    ),
    (
        "plain-model",
        r#"{"max_tokens":1.5}"#,
        "{}",
        r#"{"max_tokens":2}"#,
    ),
    (
        "plain-model",
        r#"{"max_tokens":2.5}"#,
        "{}",
        r#"{"max_tokens":2}"#,
    ),
    (
        "plain-model",
        r#"{"max_completion_tokens":3.5}"#,
        "{}",
        r#"{"max_tokens":4}"#,
    ),
    (
        "plain-model",
        r#"{"max_tokens":0.4}"#,
        "{}",
        r#"{"max_tokens":1}"#,
    ),
    ("plain-model", r#"{"seed":7}"#, "{}", "{}"),
    (
        "plain-model",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000}}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000},"max_tokens":5096}"#,
    ),
    (
        "plain-model",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000},"max_tokens":2000}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000},"max_tokens":2000}"#,
    ),
    (
        "plain-model",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000},"max_completion_tokens":2000}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":1000},"max_tokens":2000}"#,
    ),
];

async fn max_token_option_precedence() {
    check_maps(&MAX_TOKEN_CASES).await;
    for field in ["max_tokens", "max_completion_tokens"] {
        let request = format!(r#"{{"{field}":"many"}}"#);
        let error = refused("plain-model", &request).await;
        assert!(
            matches!(error, ParamsError::NotNumber { field: named } if named == field),
            "{error:?}"
        );
    }
}

const REASONING_CASES: [MapCase; 11] = [
    (
        "plain-model",
        r#"{"reasoning_effort":"none"}"#,
        r#"{"thinking":{"type":"enabled","budget_tokens":1024},"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
        "{}",
    ),
    (
        "plain-model",
        r#"{"reasoning_effort":{"effort":"low"}}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":1024},"max_tokens":5120}"#,
    ),
    (
        "plain-model",
        r#"{"reasoning_effort":"high"}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":4096},"max_tokens":8192}"#,
    ),
    ("plain-model", r#"{"reasoning_effort":42}"#, "{}", "{}"),
    (
        "plain-model",
        r#"{"reasoning_effort":{"level":"low"}}"#,
        "{}",
        "{}",
    ),
    (
        "adaptive-model",
        r#"{"reasoning_effort":"minimal"}"#,
        r#"{"output_config":{"format":{"type":"json_schema"}}}"#,
        r#"{"thinking":{"type":"adaptive","display":"summarized"},"output_config":{"effort":"low"}}"#,
    ),
    (
        "adaptive-model",
        r#"{"reasoning_effort":"xhigh"}"#,
        "{}",
        r#"{"thinking":{"type":"adaptive","display":"summarized"},"output_config":{"effort":"xhigh"}}"#,
    ),
    (
        "adaptive-model",
        r#"{"reasoning_effort":"none"}"#,
        r#"{"output_config":{"effort":"max"}}"#,
        "{}",
    ),
    (
        "adaptive-model",
        r#"{"reasoning_effort":"medium"}"#,
        "{}",
        r#"{"thinking":{"type":"adaptive","display":"summarized"},"output_config":{"effort":"medium"}}"#,
    ),
    (
        "adaptive-model",
        r#"{"reasoning_effort":"high"}"#,
        "{}",
        r#"{"thinking":{"type":"adaptive","display":"summarized"},"output_config":{"effort":"high"}}"#,
    ),
    (
        "adaptive-model",
        r#"{"reasoning_effort":"max"}"#,
        "{}",
        r#"{"thinking":{"type":"adaptive","display":"summarized"},"output_config":{"effort":"max"}}"#,
    ),
];

async fn reasoning_none_clears_thinking() {
    check_maps(&REASONING_CASES).await;
    let legacy = refused("plain-model", r#"{"reasoning_effort":"bogus"}"#).await;
    assert!(matches!(legacy, ParamsError::Reasoning(_)), "{legacy:?}");
    assert_eq!(legacy.status_code(), Some(400));
    let adaptive = refused("adaptive-model", r#"{"reasoning_effort":"bogus"}"#).await;
    assert!(matches!(adaptive, ParamsError::Effort(_)), "{adaptive:?}");
    assert_eq!(adaptive.status_code(), Some(400));
}

const OUTPUT_CASES: [OutputCase; 9] = [
    (
        "plain-model",
        true,
        r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
        FORMAT_ONLY,
        FORMAT_ONLY,
        true,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":{"effort":"high"}}"#,
        "null",
        "null",
        true,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":{"format":null,"effort":"high"}}"#,
        "null",
        "null",
        true,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":{"format":{"type":"json_schema"}}}"#,
        FORMAT_ONLY,
        FORMAT_ONLY,
        false,
    ),
    (
        "effort-model",
        true,
        r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
        EFFORT_AND_FORMAT,
        EFFORT_AND_FORMAT,
        false,
    ),
    (
        "plain-model",
        false,
        r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
        EFFORT_AND_FORMAT,
        EFFORT_AND_FORMAT,
        false,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":null}"#,
        "null",
        "null",
        false,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":{}}"#,
        "{}",
        "{}",
        false,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":"x"}"#,
        r#""x""#,
        r#""x""#,
        false,
    ),
];

async fn output_format_with_effort_drop_warning() {
    for (model, global, source, optional, data, warns) in OUTPUT_CASES {
        let (guard, warnings) = capture_warnings();
        let result = output_run(model, global, source).await.expect("applies");
        drop(guard);
        assert_eq!(result, (json(optional), json(data)), "{model} {source}");
        let expected = warns.then(|| DROP_OUTPUT_CONFIG_WARNING.replacen("%s", model, 1));
        assert_eq!(warnings.messages().first().cloned(), expected, "{source}");
    }
    let (absent, _) = output_run("plain-model", true, "{}")
        .await
        .expect("applies");
    assert_eq!(absent, Json::Null);
}

// model, output_config sent, the value the error names
const BAD_EFFORTS: [(&str, &str, &str); 3] = [
    (
        "effort-model",
        r#"{"output_config":{"effort":"bogus"}}"#,
        "'bogus'",
    ),
    ("effort-model", r#"{"output_config":{"effort":5}}"#, "5"),
    ("plain-model", r#"{"output_config":{"effort":""}}"#, "''"),
];

// model, output_config sent, the level the message names
const UNSUPPORTED_EFFORTS: [(&str, &str, &str); 2] = [
    (
        "plain-model",
        r#"{"output_config":{"effort":"xhigh","format":{"type":"json_schema"}}}"#,
        "xhigh",
    ),
    (
        "plain-model",
        r#"{"output_config":{"effort":"max"}}"#,
        "max",
    ),
];

async fn invalid_output_effort() {
    for (model, source, value) in BAD_EFFORTS {
        let error = output_run(model, false, source).await.expect_err(source);
        assert!(
            matches!(&error, ParamsError::InvalidEffort { value: named } if named == value),
            "{source}: {error:?}"
        );
        assert_eq!(error.status_code(), Some(400));
    }
    for (model, source, level) in UNSUPPORTED_EFFORTS {
        let error = output_run(model, false, source).await.expect_err(source);
        let expected =
            format!("effort='{level}' is not supported by this model. Got model: {model}");
        assert!(
            matches!(&error, ParamsError::EffortUnsupported { message } if *message == expected),
            "{source}: {error:?}"
        );
        assert_eq!(error.status_code(), Some(400));
    }
    for level in ["high", "medium", "low", "xhigh", "max"] {
        let config = format!(r#"{{"effort":"{level}","note":"kept"}}"#);
        let source = format!(r#"{{"output_config":{config}}}"#);
        let kept = output_run("effort-model", false, &source)
            .await
            .expect("a supported effort");
        assert_eq!(kept, (json(&config), json(&config)), "{level}");
    }
    let format_only = r#"{"output_config":{"format":{"type":"json_schema"}}}"#;
    let kept = output_run("plain-model", false, format_only)
        .await
        .expect("a format needs no effort capability");
    assert_eq!(kept, (json(FORMAT_ONLY), json(FORMAT_ONLY)));
}

async fn speed_drop_policy() {
    let error = refused("plain-model", r#"{"speed":"fast"}"#).await;
    assert!(matches!(error, ParamsError::Speed(_)), "{error:?}");
    assert_eq!(error.status_code(), Some(400));
    for policy in [PER_CALL, GLOBAL] {
        let (guard, warnings) = capture_warnings();
        let mapped = map_with("plain-model", r#"{"speed":"fast"}"#, policy).await;
        drop(guard);
        assert!(mapped.is_empty(), "{mapped:?}");
        assert_eq!(
            warnings.messages().first().cloned(),
            Some(DROP_UNSUPPORTED_SPEED_WARNING.replacen("%s", "plain-model", 1))
        );
    }
    check_maps(&[
        (
            "fast-model",
            r#"{"speed":"fast"}"#,
            "{}",
            r#"{"speed":"fast"}"#,
        ),
        ("plain-model", r#"{"speed":5}"#, "{}", "{}"),
    ])
    .await;
}

const ADAPTIVE_CASES: [MapCase; 8] = [
    (
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":100000}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":2048},"max_tokens":100000}"#,
    ),
    (
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":1500}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":1499},"max_tokens":1500}"#,
    ),
    (
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":1025}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":1024},"max_tokens":1025}"#,
    ),
    (
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_completion_tokens":1500,"max_tokens":100000}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":1499},"max_tokens":100000}"#,
    ),
    (
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_completion_tokens":0,"max_tokens":1500}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":1499},"max_tokens":1500}"#,
    ),
    (
        "plain-model",
        r#"{"thinking":{"type":"adaptive"}}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":2048},"max_tokens":6144}"#,
    ),
    (
        "adaptive-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":1000}"#,
        "{}",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":1000}"#,
    ),
    (
        "adaptive-model",
        r#"{"thinking":{"type":"enabled","budget_tokens":2048}}"#,
        "{}",
        r#"{"thinking":{"type":"adaptive"},"output_config":{"effort":"medium"}}"#,
    ),
];

async fn adaptive_to_legacy_cap() {
    check_maps(&ADAPTIVE_CASES).await;
    for limit in [1000, 1024] {
        let (guard, warnings) = capture_warnings();
        let request = format!(r#"{{"thinking":{{"type":"adaptive"}},"max_tokens":{limit}}}"#);
        let mapped = map_with("plain-model", &request, NONE).await;
        drop(guard);
        let expected = json(&format!(r#"{{"max_tokens":{limit}}}"#));
        assert_eq!(Json::Object(mapped), expected, "{limit}");
        assert_eq!(
            warnings.messages().first().cloned(),
            Some(DROP_ADAPTIVE_THINKING_WARNING.replacen("%s", "plain-model", 1))
        );
    }
}

const REWRITE_REQUEST: &str = r#"{"tools":[{"type":"function","function":{"name":"a","parameters":{"type":"object","properties":{}}}}],"tool_choice":"required","parallel_tool_calls":false,"stream":true,"stop":"END","temperature":0.5,"top_p":0.9,"user":"user-1","extra_headers":{"x":"1"},"cache_control":{"type":"ephemeral"},"context_management":[{"type":"compaction","compact_threshold":1000}]}"#;

const REWRITTEN: [(&str, &str); 8] = [
    (
        "tool_choice",
        r#"{"type":"any","disable_parallel_tool_use":true}"#,
    ),
    ("stream", "true"),
    ("stop_sequences", r#"["END"]"#),
    ("temperature", "0.5"),
    ("top_p", "0.9"),
    ("metadata", r#"{"user_id":"user-1"}"#),
    ("extra_headers", r#"{"x":"1"}"#),
    ("cache_control", r#"{"type":"ephemeral"}"#),
];

async fn parameter_body_rewrites() {
    let mapped = map_with("plain-model", REWRITE_REQUEST, NONE).await;
    for (key, value) in REWRITTEN {
        assert_eq!(mapped.get(key), Some(&json(value)), "{key}");
    }
    assert!(matches!(mapped.get("tools"), Some(Json::List(tools)) if tools.len() == 1));
    assert!(matches!(
        mapped.get("context_management"),
        Some(Json::Object(_))
    ));
    assert_eq!(mapped.len(), 10, "{mapped:?}");
    check_maps(&[
        ("plain-model", r#"{"stream":false,"user":"someone@example.com","extra_headers":null,"cache_control":"ephemeral"}"#, "{}", r#"{"extra_headers":null}"#),
        ("plain-model", r#"{"stop":["END","  "]}"#, "{}", r#"{"stop_sequences":["END","  "]}"#),
    ])
    .await;
    let stopped = map_with("plain-model", r#"{"stop":["END","  "]}"#, GLOBAL).await;
    assert_eq!(stopped.get("stop_sequences"), Some(&json(r#"["END"]"#)));
    let search = r#"{"web_search_options":{"search_context_size":"low"}}"#;
    let searched = try_map("plain-model", search, r#"{"tools":[{"name":"a"}]}"#, NONE)
        .await
        .expect("the search tool appends");
    assert!(matches!(searched.get("tools"), Some(Json::List(tools)) if tools.len() == 2));
}

fn format_request(extra: &str) -> String {
    format!(r#"{{{extra}"response_format":{FORMAT}}}"#)
}

async fn structured_format_selection() {
    let native = map_with("native-model", &format_request(""), NONE).await;
    assert_eq!(native.get("json_mode"), Some(&Json::Bool(true)));
    assert!(matches!(native.get("output_format"), Some(Json::Object(_))));
    assert!(native.get("tools").is_none());
    let text = r#"{"response_format":{"type":"text"}}"#;
    let native_text = map_with("native-model", text, NONE).await;
    assert_eq!(Json::Object(native_text), json(r#"{"json_mode":true}"#));
    let fallback = map_with("plain-model", &format_request(""), NONE).await;
    let forced = json(r#"{"name":"json_tool_call","type":"tool"}"#);
    assert_eq!(fallback.get("tool_choice"), Some(&forced));
    assert_eq!(fallback.get("json_mode"), Some(&Json::Bool(true)));
    assert!(matches!(fallback.get("tools"), Some(Json::List(tools)) if tools.len() == 1));
    let thinking = format_request(r#""reasoning_effort":"low","#);
    let thinking = map_with("plain-model", &thinking, NONE).await;
    assert!(thinking.get("tool_choice").is_none());
    assert!(matches!(thinking.get("tools"), Some(Json::List(tools)) if tools.len() == 1));
    let unforced = map_with("no-forced-tools", &format_request(""), NONE).await;
    assert!(unforced.get("tool_choice").is_none());
    assert_eq!(unforced.get("json_mode"), Some(&Json::Bool(true)));
    let plain_text = map_with("plain-model", text, NONE).await;
    assert!(plain_text.is_empty(), "{plain_text:?}");
    let later = format_request(r#""tool_choice":"auto","#);
    let replaced = map_with("plain-model", &later, NONE).await;
    assert_eq!(
        replaced.get("tool_choice"),
        Some(&json(r#"{"type":"auto"}"#))
    );
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
#[case::max_token_option_precedence(Scenario::MaxTokenOptionPrecedence)]
#[case::reasoning_none_clears_thinking(Scenario::ReasoningNoneClearsThinking)]
#[case::output_format_with_effort_drop_warning(Scenario::OutputFormatWithEffortDropWarning)]
#[case::invalid_output_effort(Scenario::InvalidOutputEffort)]
#[case::speed_drop_policy(Scenario::SpeedDropPolicy)]
#[case::adaptive_to_legacy_cap(Scenario::AdaptiveToLegacyCap)]
#[case::parameter_body_rewrites(Scenario::ParameterBodyRewrites)]
#[case::structured_format_selection(Scenario::StructuredFormatSelection)]
#[tokio::test]
async fn params_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::MaxTokenOptionPrecedence => max_token_option_precedence().await,
        Scenario::ReasoningNoneClearsThinking => reasoning_none_clears_thinking().await,
        Scenario::OutputFormatWithEffortDropWarning => {
            output_format_with_effort_drop_warning().await;
        }
        Scenario::InvalidOutputEffort => invalid_output_effort().await,
        Scenario::SpeedDropPolicy => speed_drop_policy().await,
        Scenario::AdaptiveToLegacyCap => adaptive_to_legacy_cap().await,
        Scenario::ParameterBodyRewrites => parameter_body_rewrites().await,
        Scenario::StructuredFormatSelection => structured_format_selection().await,
    }
}
