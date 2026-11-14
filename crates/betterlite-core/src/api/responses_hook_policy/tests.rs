#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_context::agentic_callback::{AgenticCallback, AgenticOverrides};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::agentic::get_agentic_loop_settings;
use super::callbacks::{
    CallbackLists, CallbackValue, DeploymentHookDispatcher, NamedCallbackResolver, PolicyCallback,
    ResponsesHookOutcome, ResponsesHookRequest, custom_logger_callbacks,
    has_agentic_completion_hook, has_pre_call_deployment_hook,
    run_sync_responses_pre_call_deployment_hook,
};
use super::error::{AgenticFollowupKwargsError, AgenticLoopSafetyError, DeploymentHookError};
use super::followup::{AgenticFollowupInput, build_agentic_followup_kwargs};
use super::python_value::text_repr;
use super::safety::{
    RefusedToolIdentifiers, can_replace_turn_with_terminal_response, check_agentic_loop_safety,
    fingerprint_agentic_tools, is_refused_tool_use_block, refused_agentic_tool_identifiers,
};

#[derive(Default)]
struct ScriptedDispatcher {
    result: Option<IndexMap<String, Json>>,
    calls: Mutex<Vec<(IndexMap<String, Json>, String)>>,
}

#[async_trait]
impl DeploymentHookDispatcher for ScriptedDispatcher {
    async fn pre_call_deployment_hook(
        &self,
        request: IndexMap<String, Json>,
        call_type: &str,
    ) -> Result<Option<IndexMap<String, Json>>, DeploymentHookError> {
        self.calls
            .lock()
            .unwrap()
            .push((request, call_type.to_owned()));
        Ok(self.result.clone())
    }
}

struct StaticAgentic {
    overrides: AgenticOverrides,
}

impl AgenticCallback for StaticAgentic {
    fn callback_name(&self) -> &str {
        "static"
    }

    fn overrides(&self) -> AgenticOverrides {
        self.overrides
    }
}

#[derive(Default)]
struct FixedResolver {
    named: BTreeMap<String, PolicyCallback>,
}

impl NamedCallbackResolver for FixedResolver {
    fn resolve(&self, name: &str) -> Option<PolicyCallback> {
        self.named.get(name).cloned()
    }
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn integer(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).unwrap())
}

fn list(items: Vec<Json>) -> Json {
    Json::List(items)
}

fn object(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn map(entries: Vec<(&str, Json)>) -> IndexMap<String, Json> {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn policy(name: &str, overrides: bool) -> PolicyCallback {
    PolicyCallback {
        name: name.to_owned(),
        overrides_pre_call_deployment_hook: overrides,
        agentic: None,
    }
}

fn agentic_policy(name: &str, should_run: bool) -> PolicyCallback {
    let agentic: Arc<dyn AgenticCallback> = Arc::new(StaticAgentic {
        overrides: AgenticOverrides {
            should_run,
            ..AgenticOverrides::default()
        },
    });
    PolicyCallback {
        name: name.to_owned(),
        overrides_pre_call_deployment_hook: false,
        agentic: Some(agentic),
    }
}

fn hook_request() -> ResponsesHookRequest {
    ResponsesHookRequest {
        model: "gpt-4o".to_owned(),
        input: text("hello"),
        custom_llm_provider: "openai".to_owned(),
        optional_params: map(vec![
            ("temperature", integer(1)),
            ("tools", list(Vec::new())),
        ]),
        generic_params: map(vec![("api_key", text("old"))]),
    }
}

fn original_outcome() -> ResponsesHookOutcome {
    ResponsesHookOutcome {
        model: "gpt-4o".to_owned(),
        input: text("hello"),
        custom_llm_provider: "openai".to_owned(),
        optional_params: map(vec![
            ("temperature", integer(1)),
            ("tools", list(Vec::new())),
        ]),
        generic_params: map(vec![("api_key", text("old"))]),
    }
}

async fn hook_outcome(dispatcher: &ScriptedDispatcher, overrides: bool) -> ResponsesHookOutcome {
    let global = [CallbackValue::Object(policy("a", overrides))];
    let callbacks = CallbackLists {
        global: &global,
        dynamic: None,
    };
    run_sync_responses_pre_call_deployment_hook(
        hook_request(),
        &callbacks,
        &FixedResolver::default(),
        dispatcher,
    )
    .await
    .unwrap()
}

fn detection_counts_inherited_and_resolved() {
    let inherited = policy("a", true);
    let plain = policy("b", false);
    let global = [
        CallbackValue::Object(inherited.clone()),
        CallbackValue::Named("resolved".to_owned()),
        CallbackValue::Named("missing".to_owned()),
        CallbackValue::Other,
    ];
    let dynamic = [
        CallbackValue::Object(inherited.clone()),
        CallbackValue::Object(plain.clone()),
    ];
    let resolver = FixedResolver {
        named: BTreeMap::from([("resolved".to_owned(), inherited)]),
    };
    let callbacks = CallbackLists {
        global: &global,
        dynamic: Some(&dynamic),
    };
    let callbacks = custom_logger_callbacks(&callbacks, &resolver);
    let names: Vec<&str> = callbacks
        .iter()
        .map(|callback| callback.name.as_str())
        .collect();
    assert_eq!(names, ["a", "a", "a", "b"]);
    let plain_global = [CallbackValue::Object(plain)];
    let plain_lists = CallbackLists {
        global: &plain_global,
        dynamic: None,
    };
    let all = CallbackLists {
        global: &global,
        dynamic: Some(&dynamic),
    };
    assert!(has_pre_call_deployment_hook(&all, &resolver));
    assert!(!has_pre_call_deployment_hook(&plain_lists, &resolver));
}

fn overlay_result() -> IndexMap<String, Json> {
    map(vec![
        ("model", Json::Null),
        ("input", list(Vec::new())),
        ("custom_llm_provider", Json::Null),
        ("temperature", integer(0)),
        ("api_key", text("new")),
    ])
}

#[tokio::test]
async fn deployment_hook_merge() {
    detection_counts_inherited_and_resolved();
    let dispatcher = ScriptedDispatcher {
        result: Some(overlay_result()),
        ..ScriptedDispatcher::default()
    };
    let outcome = hook_outcome(&dispatcher, true).await;
    assert_eq!(outcome.model, "None");
    assert_eq!(outcome.custom_llm_provider, "None");
    assert_eq!(outcome.input, list(Vec::new()));
    assert_eq!(
        outcome.optional_params.get("temperature"),
        Some(&integer(0))
    );
    assert_eq!(
        outcome.optional_params.get("tools"),
        Some(&list(Vec::new()))
    );
    assert_eq!(outcome.generic_params.get("api_key"), Some(&text("new")));
    let calls = dispatcher.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "responses");
    assert_eq!(calls[0].0.get("temperature"), Some(&integer(1)));
    assert_eq!(calls[0].0.get("model"), Some(&text("gpt-4o")));
    assert_eq!(calls[0].0.get("custom_llm_provider"), Some(&text("openai")));
}

#[tokio::test]
async fn deployment_hook_merge_preserves_missing_and_skips_without_override() {
    let partial = ScriptedDispatcher {
        result: Some(map(vec![("model", text("m"))])),
        ..ScriptedDispatcher::default()
    };
    let outcome = hook_outcome(&partial, true).await;
    assert_eq!(outcome.model, "m");
    assert_eq!(outcome.custom_llm_provider, "openai");
    assert_eq!(outcome.input, text("hello"));
    assert_eq!(outcome.generic_params.get("api_key"), Some(&text("old")));
    assert_eq!(
        outcome.optional_params.get("temperature"),
        Some(&integer(1))
    );

    let null = ScriptedDispatcher::default();
    assert_eq!(hook_outcome(&null, true).await, original_outcome());

    let unrun = ScriptedDispatcher {
        result: Some(overlay_result()),
        ..ScriptedDispatcher::default()
    };
    assert_eq!(hook_outcome(&unrun, false).await, original_outcome());
    assert!(unrun.calls.lock().unwrap().is_empty());
}

fn ceiling_kwargs(value: Option<Json>) -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::new();
    if let Some(value) = value {
        kwargs.insert("max_agentic_loops".to_owned(), value);
    }
    kwargs
}

fn depth_kwargs(value: Json) -> IndexMap<String, Json> {
    let mut kwargs = IndexMap::new();
    kwargs.insert("_agentic_loop_depth".to_owned(), value);
    kwargs
}

fn accepted_ceilings() {
    let accepted: Vec<(Option<Json>, u32)> = vec![
        (None, 3),
        (Some(Json::Null), 3),
        (Some(integer(5)), 5),
        (Some(float(5.0)), 5),
        (Some(text(" 5 ")), 5),
        (Some(text("+5")), 5),
        (Some(text("1_000")), 1000),
    ];
    for (value, expected) in accepted {
        let settings = get_agentic_loop_settings(&ceiling_kwargs(value)).unwrap();
        assert_eq!(settings.max_loops, expected);
    }
}

fn rejected_ceilings() {
    let not_integers = vec![
        (
            Json::Bool(true),
            "litellm_params.max_agentic_loops must be an integer, got True",
        ),
        (
            float(1.5),
            "litellm_params.max_agentic_loops must be an integer, got 1.5",
        ),
        (
            text("three"),
            "litellm_params.max_agentic_loops must be an integer, got 'three'",
        ),
        (
            text("nan"),
            "litellm_params.max_agentic_loops must be an integer, got 'nan'",
        ),
        (
            text("inf"),
            "litellm_params.max_agentic_loops must be an integer, got 'inf'",
        ),
    ];
    for (value, message) in not_integers {
        let error = get_agentic_loop_settings(&ceiling_kwargs(Some(value))).unwrap_err();
        assert_eq!(error.python_class(), "TypeError");
        assert_eq!(error.to_string(), message);
    }
    let below = get_agentic_loop_settings(&ceiling_kwargs(Some(integer(0)))).unwrap_err();
    assert_eq!(below.python_class(), "ValueError");
    assert_eq!(
        below.to_string(),
        "litellm_params.max_agentic_loops must be at least 1, got 0"
    );
    let negative = get_agentic_loop_settings(&ceiling_kwargs(Some(integer(-1)))).unwrap_err();
    assert_eq!(
        negative.to_string(),
        "litellm_params.max_agentic_loops must be at least 1, got -1"
    );
}

#[test]
fn agentic_ceiling_and_depth() {
    accepted_ceilings();
    rejected_ceilings();
    depth_cases();
    fingerprint_cases();
    repr_cases();
}

fn repr_cases() {
    assert_eq!(text_repr("it's"), "\"it's\"");
    assert_eq!(text_repr("a \"b\""), "'a \"b\"'");
    assert_eq!(text_repr("it's \"x\""), "'it\\'s \"x\"'");
    assert_eq!(text_repr("\u{1}"), "'\\x01'");
}

fn depth_cases() {
    let absent = get_agentic_loop_settings(&IndexMap::new()).unwrap();
    assert_eq!(absent.depth, 0);
    assert_eq!(
        get_agentic_loop_settings(&depth_kwargs(Json::Null))
            .unwrap()
            .depth,
        0
    );
    assert_eq!(
        get_agentic_loop_settings(&depth_kwargs(integer(0)))
            .unwrap()
            .depth,
        0
    );
    assert_eq!(
        get_agentic_loop_settings(&depth_kwargs(text("2")))
            .unwrap()
            .depth,
        2
    );
    assert_eq!(
        get_agentic_loop_settings(&depth_kwargs(integer(7)))
            .unwrap()
            .depth,
        7
    );
    assert_eq!(
        get_agentic_loop_settings(&depth_kwargs(float(2.5)))
            .unwrap()
            .depth,
        2
    );
    assert_eq!(
        get_agentic_loop_settings(&depth_kwargs(float(-2.9)))
            .unwrap()
            .depth,
        -2
    );
    assert_eq!(
        get_agentic_loop_settings(&depth_kwargs(Json::Bool(true)))
            .unwrap()
            .depth,
        1
    );
    assert_eq!(
        get_agentic_loop_settings(&depth_kwargs(text("")))
            .unwrap()
            .depth,
        0
    );
    let malformed = get_agentic_loop_settings(&depth_kwargs(text("three"))).unwrap_err();
    assert_eq!(malformed.python_class(), "ValueError");
    assert_eq!(
        malformed.to_string(),
        "invalid literal for int() with base 10: 'three'"
    );
    let wrong = get_agentic_loop_settings(&depth_kwargs(list(vec![integer(1)]))).unwrap_err();
    assert_eq!(wrong.python_class(), "TypeError");
    assert_eq!(
        wrong.to_string(),
        "int() argument must be a string, a bytes-like object or a real number, not 'list'"
    );
}

fn fingerprint_cases() {
    assert!(
        get_agentic_loop_settings(&IndexMap::new())
            .unwrap()
            .fingerprints
            .is_empty()
    );
    let mut kwargs = IndexMap::new();
    kwargs.insert("_agentic_loop_fingerprints".to_owned(), Json::Null);
    assert!(
        get_agentic_loop_settings(&kwargs)
            .unwrap()
            .fingerprints
            .is_empty()
    );
    kwargs.insert("_agentic_loop_fingerprints".to_owned(), list(Vec::new()));
    assert!(
        get_agentic_loop_settings(&kwargs)
            .unwrap()
            .fingerprints
            .is_empty()
    );
    kwargs.insert(
        "_agentic_loop_fingerprints".to_owned(),
        list(vec![text("a")]),
    );
    assert_eq!(
        get_agentic_loop_settings(&kwargs).unwrap().fingerprints,
        vec![text("a")]
    );
    kwargs.insert("_agentic_loop_fingerprints".to_owned(), integer(0));
    assert!(
        get_agentic_loop_settings(&kwargs)
            .unwrap()
            .fingerprints
            .is_empty()
    );
    kwargs.insert("_agentic_loop_fingerprints".to_owned(), float(0.0));
    assert!(
        get_agentic_loop_settings(&kwargs)
            .unwrap()
            .fingerprints
            .is_empty()
    );
    kwargs.insert("_agentic_loop_fingerprints".to_owned(), float(1.5));
    let float_history = get_agentic_loop_settings(&kwargs).unwrap_err();
    assert_eq!(float_history.to_string(), "'float' object is not iterable");
    kwargs.insert("_agentic_loop_fingerprints".to_owned(), integer(2));
    let integer_history = get_agentic_loop_settings(&kwargs).unwrap_err();
    assert_eq!(integer_history.to_string(), "'int' object is not iterable");
}

fn refused_identity() {
    let calls = list(vec![
        object(vec![
            ("id", text("a")),
            ("name", text("lookup")),
            ("call_id", integer(1)),
        ]),
        object(vec![("call_id", text("c")), ("name", integer(2))]),
        text("not a call"),
    ]);
    let refused = refused_agentic_tool_identifiers(&calls);
    assert_eq!(
        refused.ids,
        BTreeSet::from(["a".to_owned(), "c".to_owned()])
    );
    assert_eq!(refused.names, BTreeSet::from(["lookup".to_owned()]));
    let wrapped = object(vec![("tool_calls", calls)]);
    assert_eq!(refused_agentic_tool_identifiers(&wrapped), refused);
    let unrelated = object(vec![("other", integer(1))]);
    assert_eq!(
        refused_agentic_tool_identifiers(&unrelated),
        RefusedToolIdentifiers::default()
    );
}

fn block_matching() {
    let ids = BTreeSet::from(["a".to_owned()]);
    let names = BTreeSet::from(["lookup".to_owned()]);
    let other_id = object(vec![
        ("type", text("tool_use")),
        ("id", text("b")),
        ("name", text("lookup")),
    ]);
    assert!(!is_refused_tool_use_block(&other_id, &ids, &names));
    let matching = object(vec![
        ("type", text("tool_use")),
        ("id", text("a")),
        ("name", text("other")),
    ]);
    assert!(is_refused_tool_use_block(&matching, &ids, &names));
    let id_less = object(vec![("type", text("tool_use")), ("name", text("lookup"))]);
    assert!(is_refused_tool_use_block(&id_less, &ids, &names));
    let text_block = object(vec![("type", text("text")), ("name", text("lookup"))]);
    assert!(!is_refused_tool_use_block(&text_block, &ids, &names));
    assert!(!is_refused_tool_use_block(&text("x"), &ids, &names));
}

fn fingerprints_and_safety() {
    let tool = object(vec![("name", text("x")), ("id", text("a"))]);
    let reordered = object(vec![("id", text("a")), ("name", text("x"))]);
    assert_eq!(
        fingerprint_agentic_tools(&tool),
        r#"[{"id": "a", "name": "x"}]"#.to_owned()
    );
    assert_eq!(
        fingerprint_agentic_tools(&tool),
        fingerprint_agentic_tools(&reordered)
    );
    let fingerprint = fingerprint_agentic_tools(&tool);
    let repeated =
        check_agentic_loop_safety(&tool, &[text(&fingerprint)], 3, 3, "gpt-4o").unwrap_err();
    assert_eq!(repeated, AgenticLoopSafetyError::RepeatedFingerprint);
    assert_eq!(repeated.python_class(), "AgenticLoopSafetyError");
    let exhausted = check_agentic_loop_safety(&tool, &[], 3, 3, "gpt-4o").unwrap_err();
    assert_eq!(
        exhausted,
        AgenticLoopSafetyError::MaxLoopsExceeded {
            max_loops: 3,
            model: "gpt-4o".to_owned(),
        }
    );
    assert_eq!(
        exhausted.to_string(),
        "Exceeded max_agentic_loops=3 for model=gpt-4o"
    );
    assert_eq!(
        check_agentic_loop_safety(&tool, &[], 2, 3, "gpt-4o").unwrap(),
        fingerprint
    );
}

fn terminal_surface() {
    assert!(can_replace_turn_with_terminal_response(
        false,
        "anthropic_messages"
    ));
    assert!(!can_replace_turn_with_terminal_response(
        true,
        "anthropic_messages"
    ));
    assert!(!can_replace_turn_with_terminal_response(false, "responses"));
}

#[test]
fn agentic_safety_and_tool_identity() {
    fingerprints_and_safety();
    refused_identity();
    block_matching();
    terminal_surface();
}

fn agentic_detection_resolves_strings() {
    let agentic = agentic_policy("agentic", true);
    let global = [
        CallbackValue::Object(agentic_policy("plain", false)),
        CallbackValue::Named("agentic".to_owned()),
    ];
    let dynamic = [CallbackValue::Object(agentic.clone())];
    let resolver = FixedResolver {
        named: BTreeMap::from([("agentic".to_owned(), agentic)]),
    };
    let callbacks = CallbackLists {
        global: &global,
        dynamic: Some(&dynamic),
    };
    assert!(has_agentic_completion_hook(&callbacks, &resolver));
    let off = [CallbackValue::Object(agentic_policy("off", false))];
    let off_lists = CallbackLists {
        global: &off,
        dynamic: None,
    };
    assert!(!has_agentic_completion_hook(&off_lists, &resolver));
}

fn followup_inputs() -> (
    IndexMap<String, Json>,
    IndexMap<String, Json>,
    BTreeSet<String>,
) {
    let request = map(vec![
        ("model", text("old")),
        ("temperature", integer(1)),
        ("api_key", text("old")),
        ("_agentic_loop_depth", integer(50)),
        ("top_p", float(0.5)),
    ]);
    let patch = map(vec![
        ("api_key", text("new")),
        ("_agentic_loop_depth", integer(99)),
        ("max_agentic_loops", integer(9)),
        ("top_p", float(0.9)),
    ]);
    let params = BTreeSet::from([
        "model".to_owned(),
        "input".to_owned(),
        "temperature".to_owned(),
    ]);
    (request, patch, params)
}

fn followup_precedence() {
    let (request, patch, params) = followup_inputs();
    let fingerprints = [text("old")];
    let input = AgenticFollowupInput {
        request_kwargs: &request,
        patch_kwargs: &patch,
        request_params: &params,
        depth: 1,
        max_loops: 3,
        fingerprints: &fingerprints,
        fingerprint: "new",
    };
    let kwargs = build_agentic_followup_kwargs(&input).unwrap();
    let keys: Vec<&str> = kwargs.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "api_key",
            "_agentic_loop_depth",
            "top_p",
            "max_agentic_loops",
            "_agentic_loop_fingerprints",
        ]
    );
    assert_eq!(kwargs.get("api_key"), Some(&text("new")));
    assert_eq!(kwargs.get("_agentic_loop_depth"), Some(&integer(2)));
    assert_eq!(kwargs.get("top_p"), Some(&float(0.9)));
    assert_eq!(kwargs.get("max_agentic_loops"), Some(&integer(3)));
    assert_eq!(
        kwargs.get("_agentic_loop_fingerprints"),
        Some(&list(vec![text("old"), text("new")]))
    );
    assert!(!kwargs.contains_key("model"));
    assert!(!kwargs.contains_key("temperature"));
    assert_eq!(request.get("model"), Some(&text("old")));
    assert_eq!(patch.get("_agentic_loop_depth"), Some(&integer(99)));
}

fn followup_depth_overflow() {
    let empty: IndexMap<String, Json> = IndexMap::new();
    let input = AgenticFollowupInput {
        request_kwargs: &empty,
        patch_kwargs: &empty,
        request_params: &BTreeSet::new(),
        depth: i64::MAX,
        max_loops: 3,
        fingerprints: &[],
        fingerprint: "x",
    };
    assert_eq!(
        build_agentic_followup_kwargs(&input).unwrap_err(),
        AgenticFollowupKwargsError::DepthOverflow
    );
}

#[test]
fn agentic_followup_precedence() {
    agentic_detection_resolves_strings();
    followup_precedence();
    followup_depth_overflow();
}
