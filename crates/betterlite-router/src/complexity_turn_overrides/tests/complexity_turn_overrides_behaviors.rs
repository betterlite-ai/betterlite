use std::sync::Arc;

use betterlite_config::sections::complexity_turn_overrides::KeywordMatching;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::Role;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::runtime::Builder;

use super::support::{
    FakeEmbedder, base_settings, catalog, chat, json_list, json_number, json_object, json_text,
    request_kwargs, rules, semantic_settings, tier,
};
use crate::classifier_verdict::call::RequestKwargs;
use crate::complexity_turn_overrides::housekeeping::{
    cheapest_configured_tier, matched_housekeeping_sentinel,
};
use crate::complexity_turn_overrides::keywords::{LexicalRules, matched_escalation_keyword};
use crate::complexity_turn_overrides::plan::{
    apply_plan_mode_floor, matched_plan_mode_signal, plan_mode_floor_is_top_tier,
};
use crate::complexity_turn_overrides::semantic::SemanticKeywordIndex;
use crate::complexity_turn_overrides::settings::Settings;
use crate::complexity_turn_overrides::stall::detect_stalled_task;
use crate::complexity_turn_overrides::turn::{TurnRequest, resolve_turn};
use crate::semantic_router::service::CallerContext;
use crate::tier_definitions::checks::ClassifierKind;
use crate::tier_definitions::model::TierSet;

fn all_pools() -> TierSet {
    catalog(&[
        ("SIMPLE", &["simple"]),
        ("MEDIUM", &["medium"]),
        ("COMPLEX", &["complex"]),
        ("REASONING", &["reasoning"]),
    ])
}

fn floor_settings(floor: &str) -> Settings {
    let mut settings = base_settings();
    settings.plan_mode_min_tier = Some(floor.to_owned());
    settings
}

#[rstest]
#[case("SIMPLE", "COMPLEX", "COMPLEX")]
#[case("MEDIUM", "COMPLEX", "COMPLEX")]
#[case("COMPLEX", "COMPLEX", "COMPLEX")]
#[case("REASONING", "COMPLEX", "REASONING")]
fn the_plan_mode_floor_raises_only_a_lower_tier(
    #[case] decided: &str,
    #[case] floor: &str,
    #[case] expected: &str,
) {
    let applied =
        apply_plan_mode_floor(&floor_settings(floor), &all_pools(), tier(decided)).unwrap();
    assert_eq!(applied.name(), expected);
}

#[rstest]
#[case(&["SIMPLE", "MEDIUM", "COMPLEX"], true)]
#[case(&["SIMPLE", "MEDIUM", "COMPLEX", "REASONING"], false)]
#[case(&["SIMPLE", "MEDIUM"], false)]
fn the_top_tier_shortcut_counts_the_configured_pools(
    #[case] configured: &[&str],
    #[case] expected: bool,
) {
    let mut set = all_pools();
    set.pools
        .retain(|name, _pool| configured.contains(&name.as_str()));
    let found = plan_mode_floor_is_top_tier(&floor_settings("COMPLEX"), &set).unwrap();
    assert_eq!(found, expected);
}

#[test]
fn a_plan_mode_tool_or_sentinel_is_recognized() {
    let settings = floor_settings("COMPLEX");
    let tool = json_list(vec![json_object(&[("name", json_text("exit_plan_mode"))])]);
    let kwargs = request_kwargs(json_object(&[("tools", tool)]));
    let found = matched_plan_mode_signal(&settings, Some(&kwargs), None).unwrap();
    assert_eq!(found.as_deref(), Some("exit_plan_mode"));

    let body = json_object(&[(
        "system",
        json_text("You are currently running in \"Plan\" mode."),
    )]);
    let kwargs = request_kwargs(body);
    let found = matched_plan_mode_signal(&settings, Some(&kwargs), None).unwrap();
    assert_eq!(
        found.as_deref(),
        Some("You are currently running in \"Plan\" mode.")
    );

    let quiet = Settings::default();
    let kwargs = request_kwargs(json_object(&[]));
    assert_eq!(
        matched_plan_mode_signal(&quiet, Some(&kwargs), None).unwrap(),
        None
    );
}

#[test]
fn the_client_marker_set_decides_where_the_newest_ask_starts() {
    let messages = vec![
        chat(Role::User, "Plan mode is active"),
        chat(
            Role::User,
            "<environment_context>keep going</environment_context>",
        ),
    ];
    let settings = floor_settings("COMPLEX");
    let codex = IndexMap::from([(
        "metadata".to_owned(),
        json_object(&[("user_agent", json_text("codex-cli/0.20"))]),
    )]);
    let found = matched_plan_mode_signal(&settings, Some(&codex), Some(&messages)).unwrap();
    assert_eq!(found.as_deref(), Some("Plan mode is active"));
    let found = matched_plan_mode_signal(&settings, None, Some(&messages)).unwrap();
    assert_eq!(found, None);
}

#[test]
fn a_housekeeping_ask_routes_to_the_cheapest_tier_with_models() {
    let settings = Settings::default();
    let ask = "Write the title in the predominant language of the session";
    assert_eq!(
        matched_housekeeping_sentinel(&settings, Some(ask)).as_deref(),
        Some(ask)
    );
    let set = catalog(&[("SIMPLE", &[]), ("MEDIUM", &["medium"])]);
    let cheapest = cheapest_configured_tier(&set).unwrap();
    assert_eq!(
        cheapest.map(|found| found.name().to_owned()),
        Some("MEDIUM".to_owned())
    );
    let mut without_ask = settings.clone();
    without_ask.housekeeping_cheapest = false;
    assert_eq!(matched_housekeeping_sentinel(&without_ask, Some(ask)), None);
    let mut plugin = base_settings();
    plugin.classifier_type = ClassifierKind::Custom;
    assert_eq!(matched_housekeeping_sentinel(&plugin, Some(ask)), None);
}

#[test]
fn the_escalation_keyword_defaults_to_the_shouted_phrase() {
    let set = all_pools();
    let settings = Settings::default();
    assert_eq!(
        matched_escalation_keyword(&settings, &set, "please LITELLM ESCALATE now").as_deref(),
        Some("LITELLM ESCALATE")
    );
    assert_eq!(
        matched_escalation_keyword(&settings, &set, "liteLLM escalate"),
        None
    );
    let mut custom = base_settings();
    custom.escalation_keywords = Some(vec!["help me".to_owned()]);
    assert_eq!(
        matched_escalation_keyword(&custom, &set, "help me debug").as_deref(),
        Some("help me")
    );
    let mut off = base_settings();
    off.escalation_keywords = Some(Vec::new());
    assert_eq!(
        matched_escalation_keyword(&off, &set, "LITELLM ESCALATE"),
        None
    );
    let custom_tiers =
        super::support::custom_catalog(&["cheap", "deep"], &[("cheap", &["a"]), ("deep", &["b"])]);
    assert_eq!(
        matched_escalation_keyword(&settings, &custom_tiers, "LITELLM ESCALATE"),
        None
    );
}

#[test]
fn the_most_severe_lexical_rule_wins_whatever_the_rule_order() {
    let set = all_pools();
    let configured = rules(&[(&["refund"], "SIMPLE"), (&["invoice"], "COMPLEX")]);
    let compiled = LexicalRules::compile(&set, &configured).unwrap();
    let found = compiled.matches("please refund the invoice").unwrap();
    assert_eq!(found.tier.name(), "COMPLEX");
    assert_eq!(found.matched_keyword.as_deref(), Some("invoice"));
    let phrase = rules(&[(&["invoice number"], "MEDIUM")]);
    let compiled = LexicalRules::compile(&set, &phrase).unwrap();
    assert_eq!(
        compiled
            .matches("the INVOICE NUMBER is 7")
            .unwrap()
            .tier
            .name(),
        "MEDIUM"
    );
    let compiled = LexicalRules::compile(&set, &rules(&[(&["api"], "SIMPLE")])).unwrap();
    assert_eq!(compiled.matches("the capital is Paris"), None);
}

#[test]
fn a_repeated_or_failed_tool_call_marks_the_task_stalled() {
    let repeated = vec![
        tool_result("a1", true),
        assistant_tool_use("a1", "search", json_object(&[("q", json_text("x"))])),
        tool_result("a2", true),
        assistant_tool_use("a2", "search", json_object(&[("q", json_text("x"))])),
        tool_result("a3", true),
        assistant_tool_use("a3", "search", json_object(&[("q", json_text("x"))])),
    ];
    assert!(detect_stalled_task(Some(&repeated), 6, 3));

    let chat_calls = vec![assistant_calls(&[
        ("c1", "{\"q\": \"x\"}"),
        ("c2", "{\"q\": \"x\"}"),
        ("c3", "{\"q\": \"x\"}"),
    ])];
    assert!(detect_stalled_task(Some(&chat_calls), 6, 3));

    let errors = vec![
        tool_results(&[("a", true), ("b", true), ("c", true)]),
        assistant_tool_use("a", "one", Json::Null),
        assistant_tool_use("b", "two", Json::Null),
        assistant_tool_use("c", "three", Json::Null),
    ];
    assert!(detect_stalled_task(Some(&errors), 3, 3));
    assert!(!detect_stalled_task(Some(&repeated), 2, 3));
    assert!(!detect_stalled_task(None, 6, 3));
}

#[test]
fn both_tool_call_surfaces_share_one_canonical_signature() {
    let messages = vec![
        assistant_calls(&[("c1", "{\"b\": 2, \"a\": 1}")]),
        assistant_tool_use(
            "a1",
            "search",
            json_object(&[("a", json_number(1)), ("b", json_number(2))]),
        ),
    ];
    assert!(detect_stalled_task(Some(&messages), 6, 2));
}

#[test]
fn a_semantic_hit_selects_its_route_and_reports_no_keyword() {
    let settings = semantic_settings(&[(&["invoice"], "COMPLEX")], 0.5);
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("invoice", vec![1.0, 0.0]),
        ("please send an invoice", vec![1.0, 0.0]),
    ]));
    let index = SemanticKeywordIndex::new(&settings, &all_pools(), Arc::clone(&embedder)).unwrap();
    let caller = CallerContext {
        metadata: IndexMap::new(),
        litellm_metadata: IndexMap::new(),
        turn_off_message_logging: None,
        litellm_session_id: None,
        litellm_trace_id: Some("trace-1".to_owned()),
    };
    let found = runtime().block_on(index.tier_override("please send an invoice", &caller));
    assert_eq!(
        found.map(|tier| tier.name().to_owned()),
        Some("COMPLEX".to_owned())
    );
    let documents = embedder.documents();
    assert_eq!(documents.first().unwrap().texts, ["test"]);
    assert_eq!(documents.last().unwrap().texts, ["invoice"]);
    assert_eq!(
        embedder
            .queries()
            .first()
            .unwrap()
            .caller
            .litellm_trace_id
            .as_deref(),
        Some("trace-1")
    );
}

#[test]
fn a_semantic_failure_falls_back_to_no_override() {
    let settings = semantic_settings(&[(&["invoice"], "COMPLEX")], 0.5);
    let failing_build = FakeEmbedder::new(&[("test", vec![1.0, 0.0])]).failing_documents();
    let index =
        SemanticKeywordIndex::new(&settings, &all_pools(), Arc::new(failing_build)).unwrap();
    assert_eq!(
        runtime().block_on(index.tier_override("invoice", &CallerContext::default())),
        None
    );
    let failing_query =
        FakeEmbedder::new(&[("test", vec![1.0, 0.0]), ("invoice", vec![1.0, 0.0])]).failing_query();
    let index =
        SemanticKeywordIndex::new(&settings, &all_pools(), Arc::new(failing_query)).unwrap();
    assert_eq!(
        runtime().block_on(index.tier_override("invoice", &CallerContext::default())),
        None
    );
}

#[test]
fn a_build_failure_is_retried_on_the_next_turn() {
    let settings = semantic_settings(&[(&["invoice"], "COMPLEX")], 0.5);
    let embedder = Arc::new(
        FakeEmbedder::new(&[("test", vec![1.0, 0.0]), ("invoice", vec![1.0, 0.0])])
            .failing_documents(),
    );
    let index = SemanticKeywordIndex::new(&settings, &all_pools(), Arc::clone(&embedder)).unwrap();
    let first = runtime().block_on(index.tier_override("invoice", &CallerContext::default()));
    assert_eq!(first, None);
    let second = runtime().block_on(index.tier_override("invoice", &CallerContext::default()));
    assert_eq!(
        second.map(|tier| tier.name().to_owned()),
        Some("COMPLEX".to_owned())
    );
}

#[test]
fn the_lexical_matcher_wins_while_semantic_matching_is_off() {
    let configured = rules(&[(&["invoice"], "COMPLEX"), (&["refund"], "SIMPLE")]);
    let lexical = LexicalRules::compile(&all_pools(), &configured).unwrap();
    let mut settings = base_settings();
    settings.keyword_tier_rules = rules(&[(&["invoice"], "COMPLEX"), (&["refund"], "SIMPLE")]);
    let embedder = Arc::new(FakeEmbedder::new(&[
        ("test", vec![1.0, 0.0]),
        ("invoice", vec![0.0, 1.0]),
        ("refund", vec![1.0, 0.0]),
        ("invoice and refund", vec![1.0, 0.0]),
    ]));
    let mut index_settings = settings.clone();
    index_settings.keyword_matching = KeywordMatching::Semantic;
    index_settings.embedding_model = Some("fake-embed".to_owned());
    index_settings.match_threshold = 0.5;
    let semantic = SemanticKeywordIndex::new(&index_settings, &all_pools(), embedder).unwrap();
    let request = TurnRequest {
        request_kwargs: None,
        resolved_messages: None,
        messages: None,
        user_message: "invoice and refund",
        newest_ask: None,
        caller: &CallerContext::default(),
    };
    let lexical_outcome = runtime()
        .block_on(resolve_turn(
            &settings,
            &all_pools(),
            &lexical,
            None,
            &request,
        ))
        .unwrap();
    assert_eq!(
        lexical_outcome.keyword_override.unwrap().tier.name(),
        "COMPLEX"
    );
    let mut semantic_mode = settings;
    semantic_mode.keyword_matching = KeywordMatching::Semantic;
    let semantic_outcome = runtime()
        .block_on(resolve_turn(
            &semantic_mode,
            &all_pools(),
            &lexical,
            Some(&semantic),
            &request,
        ))
        .unwrap();
    let found = semantic_outcome.keyword_override.unwrap();
    assert_eq!(found.tier.name(), "SIMPLE");
    assert_eq!(found.matched_keyword, None);
}

#[test]
fn the_turn_carries_every_observation_together() {
    let mut settings = base_settings();
    settings.plan_mode_min_tier = Some("COMPLEX".to_owned());
    settings.stall_escalation_enabled = true;
    let messages = vec![
        tool_result("a1", true),
        assistant_tool_use("a1", "search", Json::Null),
        tool_result("a2", true),
        assistant_tool_use("a2", "search", Json::Null),
        tool_result("a3", true),
        assistant_tool_use("a3", "search", Json::Null),
    ];
    let lexical = LexicalRules::compile(&all_pools(), &[]).unwrap();
    let body = json_object(&[(
        "tools",
        json_list(vec![json_object(&[("name", json_text("exit_plan_mode"))])]),
    )]);
    let kwargs = request_kwargs(body);
    let request = TurnRequest {
        request_kwargs: Some(&kwargs),
        resolved_messages: None,
        messages: Some(&messages),
        user_message: "carry on LITELLM ESCALATE",
        newest_ask: Some("Write the title in the predominant language of the session"),
        caller: &CallerContext::default(),
    };
    let found = runtime()
        .block_on(resolve_turn(
            &settings,
            &all_pools(),
            &lexical,
            None,
            &request,
        ))
        .unwrap();
    assert_eq!(found.plan_mode_sentinel.as_deref(), Some("exit_plan_mode"));
    assert_eq!(found.plan_mode_floor, Some(tier("COMPLEX")));
    assert!(!found.plan_mode_floor_is_top_tier);
    assert_eq!(
        found.escalation_keyword.as_deref(),
        Some("LITELLM ESCALATE")
    );
    assert!(found.stalled);
    assert!(found.housekeeping_sentinel.is_some());
    assert_eq!(found.housekeeping_tier, Some(tier("SIMPLE")));
    assert_eq!(found.keyword_override, None);
}

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the test runtime starts")
}

fn assistant_tool_use(id: &str, name: &str, input: Json) -> Json {
    json_object(&[
        ("role", json_text("assistant")),
        (
            "content",
            json_list(vec![json_object(&[
                ("type", json_text("tool_use")),
                ("id", json_text(id)),
                ("name", json_text(name)),
                ("input", input),
            ])]),
        ),
    ])
}

fn tool_result(id: &str, is_error: bool) -> Json {
    tool_results(&[(id, is_error)])
}

fn tool_results(results: &[(&str, bool)]) -> Json {
    json_object(&[
        ("role", json_text("user")),
        (
            "content",
            json_list(
                results
                    .iter()
                    .map(|(id, is_error)| {
                        json_object(&[
                            ("type", json_text("tool_result")),
                            ("tool_use_id", json_text(id)),
                            ("is_error", Json::Bool(*is_error)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn assistant_calls(calls: &[(&str, &str)]) -> Json {
    json_object(&[
        ("role", json_text("assistant")),
        (
            "tool_calls",
            json_list(
                calls
                    .iter()
                    .map(|(id, arguments)| {
                        json_object(&[
                            ("id", json_text(id)),
                            (
                                "function",
                                json_object(&[
                                    ("name", json_text("search")),
                                    ("arguments", json_text(arguments)),
                                ]),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}
