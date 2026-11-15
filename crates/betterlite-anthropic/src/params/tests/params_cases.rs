use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::fixture::{Fixture, Session, capture_warnings};
use crate::anthropic_capability_catalog::DropParams;
use crate::anthropic_capability_catalog::forced_tool::DROP_FORCED_TOOL_WARNING;
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
    "no-forced-tools": {"litellm_provider": "anthropic", "supports_forced_tool_use": false},
    "no-sampling": {"litellm_provider": "anthropic", "supports_sampling_params": false}
}"#;

const FORMAT: &str = r#"{"type":"json_schema","json_schema":{"schema":{"type":"object"}}}"#;
const FORMAT_ONLY: &str = r#"{"format":{"type":"json_schema"}}"#;
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

// model, global drop, output_config sent, every field of the optional params and the body after, warns
type OutputCase = (&'static str, bool, &'static str, &'static str, bool);

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

// every unsupported parameter is refused with the 400 its error names
async fn gate_error(model: &str, requested: &str) -> ParamsError {
    let error = refused(model, requested).await;
    assert_eq!(error.status_code(), Some(400), "{requested}");
    error
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

// the optional params and the body after _apply_output_config, whole so a removed config reads absent
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
    Ok((Json::Object(optional), Json::Object(data)))
}

// an output_config the model can't take is refused with the 400 its error names
async fn output_error(model: &str, source: &str) -> ParamsError {
    let error = output_run(model, false, source).await.expect_err(source);
    assert_eq!(error.status_code(), Some(400), "{source}");
    error
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
        r#"{"output_config":{"format":{"type":"json_schema"}}}"#,
        true,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":{"effort":"high"}}"#,
        "{}",
        true,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":{"format":null,"effort":"high"}}"#,
        "{}",
        true,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":{"format":{"type":"json_schema"}}}"#,
        r#"{"output_config":{"format":{"type":"json_schema"}}}"#,
        false,
    ),
    (
        "effort-model",
        true,
        r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
        r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
        false,
    ),
    (
        "plain-model",
        false,
        r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
        r#"{"output_config":{"effort":"high","format":{"type":"json_schema"}}}"#,
        false,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":null}"#,
        r#"{"output_config":null}"#,
        false,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":{}}"#,
        r#"{"output_config":{}}"#,
        false,
    ),
    (
        "plain-model",
        true,
        r#"{"output_config":"x"}"#,
        r#"{"output_config":"x"}"#,
        false,
    ),
];

async fn output_format_with_effort_drop_warning() {
    // the whole maps compare too, so a removed output_config reads absent instead of null
    for (model, global, source, expected, warns) in OUTPUT_CASES {
        let (guard, warnings) = capture_warnings();
        let result = output_run(model, global, source).await.expect("applies");
        drop(guard);
        assert_eq!(result, (json(expected), json(expected)), "{model} {source}");
        let warned = warns.then(|| DROP_OUTPUT_CONFIG_WARNING.replacen("%s", model, 1));
        assert_eq!(warnings.messages().first().cloned(), warned, "{source}");
    }
    let (absent_optional, absent_data) = output_run("plain-model", true, "{}")
        .await
        .expect("applies");
    assert_eq!(absent_optional, json("{}"));
    assert_eq!(absent_data, json("{}"));
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
    refuse_invalid_efforts().await;
    refuse_unsupported_efforts().await;
    keep_supported_efforts().await;
}

const fn invalid_value(error: &ParamsError) -> Option<&str> {
    match error {
        ParamsError::InvalidEffort { value } => Some(value.as_str()),
        _ => None,
    }
}

async fn refuse_invalid_efforts() {
    for (model, source, value) in BAD_EFFORTS {
        let error = output_error(model, source).await;
        assert_eq!(invalid_value(&error), Some(value), "{source}");
    }
}

async fn refuse_unsupported_efforts() {
    for (model, source, level) in UNSUPPORTED_EFFORTS {
        let error = output_error(model, source).await;
        let expected =
            format!("effort='{level}' is not supported by this model. Got model: {model}");
        assert!(
            matches!(&error, ParamsError::EffortUnsupported { message } if *message == expected),
            "{source}: {error:?}"
        );
    }
}

async fn keep_supported_efforts() {
    for level in ["high", "medium", "low", "xhigh", "max"] {
        let config = format!(r#"{{"effort":"{level}","note":"kept"}}"#);
        let source = format!(r#"{{"output_config":{config}}}"#);
        let kept = output_run("effort-model", false, &source)
            .await
            .expect("a supported effort");
        assert_eq!(kept, (json(&source), json(&source)), "{level}");
    }
    let format_only = format!(r#"{{"output_config":{FORMAT_ONLY}}}"#);
    let kept = output_run("plain-model", false, &format_only)
        .await
        .expect("a format needs no effort capability");
    assert_eq!(kept, (json(&format_only), json(&format_only)));
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

const ADAPTIVE_CASES: [MapCase; 11] = [
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
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_completion_tokens":0}"#,
        "{}",
        r#"{"thinking":{"type":"enabled","budget_tokens":2048},"max_tokens":0}"#,
    ),
    (
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_completion_tokens":0,"max_tokens":0}"#,
        "{}",
        r#"{"max_tokens":0}"#,
    ),
    (
        "plain-model",
        r#"{"thinking":{"type":"adaptive"},"max_tokens":false}"#,
        "{}",
        r#"{"max_tokens":false}"#,
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
    for limit in ["0", "1000", "1024"] {
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
    for value in [r#""""#, "[]", "{}"] {
        let request = format!(
            r#"{{"thinking":{{"type":"adaptive"}},"max_completion_tokens":{value},"max_tokens":1500}}"#
        );
        let error = refused("plain-model", &request).await;
        assert!(
            matches!(
                error,
                ParamsError::NotNumber {
                    field: "max_completion_tokens"
                }
            ),
            "{value}: {error:?}"
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
    check_rewritten_fields().await;
    check_forced_tool_gate().await;
    check_sampling_params().await;
    check_tool_appends().await;
}

async fn check_rewritten_fields() {
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
        (
            "plain-model",
            r#"{"tool_choice":"required"}"#,
            "{}",
            r#"{"tool_choice":{"type":"any"}}"#,
        ),
    ])
    .await;
}

async fn check_forced_tool_gate() {
    // the forced-tool gate reads both a required string and a named tool choice
    for forced in [
        r#"{"tool_choice":"required"}"#,
        r#"{"tool_choice":{"type":"function","function":{"name":"a"}}}"#,
    ] {
        let error = gate_error("no-forced-tools", forced).await;
        assert!(
            matches!(error, ParamsError::ForcedTool(_)),
            "{forced}: {error:?}"
        );
    }
    for policy in [PER_CALL, GLOBAL] {
        let (guard, warnings) = capture_warnings();
        let downgraded = map_with(
            "no-forced-tools",
            r#"{"tool_choice":"required","parallel_tool_calls":false}"#,
            policy,
        )
        .await;
        drop(guard);
        assert_eq!(
            Json::Object(downgraded),
            json(r#"{"tool_choice":{"type":"auto","disable_parallel_tool_use":true}}"#)
        );
        assert_eq!(
            warnings.messages().first().cloned(),
            Some(DROP_FORCED_TOOL_WARNING.replacen("%s", "no-forced-tools", 1))
        );
    }
}

async fn check_sampling_params() {
    // the sampling gate rejects each param alone and drops both under either drop
    for rejected in [r#"{"temperature":0.5}"#, r#"{"top_p":0.9}"#] {
        let error = gate_error("no-sampling", rejected).await;
        assert!(
            matches!(error, ParamsError::Sampling(_)),
            "{rejected}: {error:?}"
        );
    }
    for policy in [PER_CALL, GLOBAL] {
        let dropped = map_with("no-sampling", r#"{"temperature":0.5,"top_p":0.9}"#, policy).await;
        assert!(dropped.is_empty(), "{dropped:?}");
    }
}

async fn check_tool_appends() {
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
    check_native_selection().await;
    check_fallback_selection().await;
    check_input_order().await;
}

async fn check_native_selection() {
    let native = map_with("native-model", &format_request(""), NONE).await;
    assert_eq!(native.get("json_mode"), Some(&Json::Bool(true)));
    assert!(matches!(native.get("output_format"), Some(Json::Object(_))));
    assert!(native.get("tools").is_none());
    let text = r#"{"response_format":{"type":"text"}}"#;
    let native_text = map_with("native-model", text, NONE).await;
    assert_eq!(Json::Object(native_text), json(r#"{"json_mode":true}"#));
    let unforced = map_with("no-forced-tools", &format_request(""), NONE).await;
    assert!(unforced.get("tool_choice").is_none());
    assert_eq!(unforced.get("json_mode"), Some(&Json::Bool(true)));
    let plain_text = map_with("plain-model", text, NONE).await;
    assert!(plain_text.is_empty(), "{plain_text:?}");
}

fn tool_count(optional: &IndexMap<String, Json>) -> Option<usize> {
    match optional.get("tools") {
        Some(Json::List(tools)) => Some(tools.len()),
        None | Some(_) => None,
    }
}

async fn check_fallback_selection() {
    let fallback = map_with("plain-model", &format_request(""), NONE).await;
    let forced = json(r#"{"name":"json_tool_call","type":"tool"}"#);
    assert_eq!(fallback.get("tool_choice"), Some(&forced));
    assert_eq!(fallback.get("json_mode"), Some(&Json::Bool(true)));
    assert_eq!(tool_count(&fallback), Some(1));
    check_thinking_fallback().await;
    // response_format first still sees the reasoning_effort in the initial non_default_params
    let ordered = format!(r#"{{"response_format":{FORMAT},"reasoning_effort":"low"}}"#);
    let ordered_mapped = map_with("plain-model", &ordered, NONE).await;
    assert_eq!(tool_count(&ordered_mapped), Some(1));
    assert_eq!(ordered_mapped.get("json_mode"), Some(&Json::Bool(true)));
    assert!(
        ordered_mapped.get("tool_choice").is_none(),
        "{ordered_mapped:?}"
    );
}

async fn check_thinking_fallback() {
    let thinking = format_request(r#""reasoning_effort":"low","#);
    let mapped = map_with("plain-model", &thinking, NONE).await;
    assert_eq!(tool_count(&mapped), Some(1));
    assert!(mapped.get("tool_choice").is_none(), "{mapped:?}");
}

async fn check_input_order() {
    let forced = json(r#"{"name":"json_tool_call","type":"tool"}"#);
    // thinking already in optional_params is not part of the initial check, so the choice stays forced
    let sent = r#"{"thinking":{"type":"enabled","budget_tokens":1024}}"#;
    let accumulated = try_map("plain-model", &format_request(""), sent, NONE)
        .await
        .expect("the format maps");
    assert_eq!(accumulated.get("tool_choice"), Some(&forced));
    assert_eq!(accumulated.get("max_tokens"), Some(&json("5120")));
    // the loop keeps input order, so a response_format after tool_choice replaces the choice
    let later = format_request(r#""tool_choice":"auto","#);
    let replaced = map_with("plain-model", &later, NONE).await;
    assert_eq!(
        replaced.get("tool_choice"),
        Some(&json(r#"{"name":"json_tool_call","type":"tool"}"#))
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
