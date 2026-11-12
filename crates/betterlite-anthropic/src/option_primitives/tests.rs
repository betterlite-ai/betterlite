#![cfg(test)]

use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::context_management::map_context_management;
use super::user_id::UserIdPolicy;
use super::{
    CUSTOM_LLM_PROVIDER, OptionDefaults, WebSearchOptions, get_max_tokens_for_model,
    map_stop_sequences, map_web_search_tool, resolved_provider,
};

const CATALOG: &str = r#"{
  "zero-limit": {"litellm_provider": "anthropic", "mode": "chat", "max_output_tokens": 0},
  "max-only": {"litellm_provider": "anthropic", "mode": "chat", "max_tokens": 7},
  "no-limits": {"litellm_provider": "anthropic", "mode": "chat"},
  "wide-limit": {"litellm_provider": "anthropic", "mode": "chat", "max_output_tokens": 18446744073709551615}
}"#;

#[derive(Clone, Copy, Debug)]
enum Case {
    DefaultMaxTokens,
    ModelOutputCapacity,
    StopSequenceFilter,
    WebSearchContextSize,
    ContextCompactionEdit,
    UserIdentifierFilter,
}

#[rstest]
#[case::default_max_tokens(Case::DefaultMaxTokens)]
#[case::model_output_capacity(Case::ModelOutputCapacity)]
#[case::stop_sequence_filter(Case::StopSequenceFilter)]
#[case::web_search_context_size(Case::WebSearchContextSize)]
#[case::context_compaction_edit(Case::ContextCompactionEdit)]
#[case::user_identifier_filter(Case::UserIdentifierFilter)]
fn option_primitives_cases(#[case] case: Case) {
    match case {
        Case::DefaultMaxTokens => default_max_tokens_case(),
        Case::ModelOutputCapacity => model_output_capacity_case(),
        Case::StopSequenceFilter => stop_sequence_filter_case(),
        Case::WebSearchContextSize => web_search_context_size_case(),
        Case::ContextCompactionEdit => context_compaction_edit_case(),
        Case::UserIdentifierFilter => user_identifier_filter_case(),
    }
}

fn catalog() -> ModelCatalog {
    ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn config_of(defaults: &OptionDefaults, model: Option<&str>) -> Json {
    defaults.get_config(model, &catalog(), 4096).unwrap()
}

fn default_max_tokens_case() {
    assert_eq!(CUSTOM_LLM_PROVIDER, "anthropic");
    assert_defaults_filter();
    assert_max_tokens_fill();
    assert_resolved_providers();
}

fn assert_defaults_filter() {
    assert_eq!(
        config_of(&OptionDefaults::default(), None),
        json(r#"{"max_tokens": 4096}"#)
    );
    let falsey = OptionDefaults {
        max_tokens: Some(0),
        stop_sequences: Some(Vec::new()),
        temperature: Some(Number::from(0_i64)),
        top_p: Some(Number::from(0_i64)),
        top_k: Some(0),
        metadata: Some(IndexMap::new()),
        system: Some(String::new()),
    };
    assert_eq!(
        config_of(&falsey, None),
        json(
            r#"{"max_tokens":0,"stop_sequences":[],"temperature":0,"top_p":0,"top_k":0,"metadata":{},"system":""}"#
        )
    );
    let labeled = OptionDefaults {
        max_tokens: Some(9),
        stop_sequences: Some(vec!["END".to_owned()]),
        temperature: Some(Number::from_f64(0.5).unwrap()),
        top_p: Some(Number::from_f64(0.9).unwrap()),
        top_k: Some(3),
        metadata: Some(IndexMap::from([("user_id".to_owned(), json(r#""abc""#))])),
        system: Some("be brief".to_owned()),
    };
    assert_eq!(
        config_of(&labeled, Some("zero-limit")),
        json(
            r#"{"max_tokens":9,"stop_sequences":["END"],"temperature":0.5,"top_p":0.9,"top_k":3,"metadata":{"user_id":"abc"},"system":"be brief"}"#
        )
    );
}

fn assert_max_tokens_fill() {
    let empty = OptionDefaults::default();
    let config = |model: Option<&str>| config_of(&empty, model);
    assert_eq!(config(None), json(r#"{"max_tokens":4096}"#));
    assert_eq!(config(Some("zero-limit")), json(r#"{"max_tokens":0}"#));
    assert_eq!(config(Some("max-only")), json(r#"{"max_tokens":7}"#));
    assert_eq!(config(Some("no-limits")), json(r#"{"max_tokens":4096}"#));
    assert_eq!(
        config(Some("missing-model")),
        json(r#"{"max_tokens":4096}"#)
    );
    let explicit = OptionDefaults {
        max_tokens: Some(11),
        stop_sequences: None,
        temperature: None,
        top_p: None,
        top_k: None,
        metadata: None,
        system: None,
    };
    assert_eq!(
        config_of(&explicit, Some("zero-limit")),
        json(r#"{"max_tokens":11}"#)
    );
}

fn assert_resolved_providers() {
    assert_eq!(resolved_provider(None), "anthropic");
    assert_eq!(resolved_provider(Some("")), "anthropic");
    assert_eq!(resolved_provider(Some("vertex_ai")), "vertex_ai");
    assert_eq!(resolved_provider(Some("anthropic")), "anthropic");
}

fn model_output_capacity_case() {
    let catalog = catalog();
    assert_eq!(get_max_tokens_for_model(&catalog, None, 4096), 4096);
    assert_eq!(
        get_max_tokens_for_model(&catalog, Some("zero-limit"), 4096),
        0
    );
    assert_eq!(
        get_max_tokens_for_model(&catalog, Some("max-only"), 4096),
        7
    );
    assert_eq!(
        get_max_tokens_for_model(&catalog, Some("no-limits"), 4096),
        4096
    );
    assert_eq!(
        get_max_tokens_for_model(&catalog, Some("missing-model"), 1234),
        1234
    );
    assert_eq!(get_max_tokens_for_model(&catalog, None, 1234), 1234);
    assert_eq!(
        get_max_tokens_for_model(&catalog, Some("wide-limit"), 1234),
        1234
    );
    assert_eq!(
        get_max_tokens_for_model(&catalog, Some("huggingface/anything"), 1234),
        1234
    );
}

fn stop_sequence_filter_case() {
    assert_stop_dropped();
    assert_stop_kept();
    assert_stop_other_shapes();
}

fn assert_stop_dropped() {
    let empty = String::new();
    assert_eq!(map_stop_sequences(&json(r#"" ""#), true).unwrap(), None);
    assert_eq!(map_stop_sequences(&json(r#""\t\n""#), true).unwrap(), None);
    assert_eq!(
        map_stop_sequences(&json(r#""\u00a0""#), true).unwrap(),
        None
    );
    assert_eq!(
        map_stop_sequences(&json(r#""\u001c""#), true).unwrap(),
        None
    );
    assert_eq!(
        map_stop_sequences(&json(r#"["END", " ", ""]"#), true).unwrap(),
        Some(vec!["END".to_owned(), empty])
    );
    assert_eq!(
        map_stop_sequences(&json(r#"[" ", "\t"]"#), true).unwrap(),
        None
    );
    assert_eq!(map_stop_sequences(&json("[]"), true).unwrap(), None);
}

fn assert_stop_kept() {
    let empty = String::new();
    assert_eq!(
        map_stop_sequences(&json(r#"" ""#), false).unwrap(),
        Some(vec![" ".to_owned()])
    );
    assert_eq!(
        map_stop_sequences(&json(r#"["END", " "]"#), false).unwrap(),
        Some(vec!["END".to_owned(), " ".to_owned()])
    );
    assert_eq!(
        map_stop_sequences(&json(r#""""#), true).unwrap(),
        Some(vec![empty.clone()])
    );
    assert_eq!(
        map_stop_sequences(&json(r#"[""]"#), true).unwrap(),
        Some(vec![empty])
    );
}

fn assert_stop_other_shapes() {
    assert_eq!(map_stop_sequences(&json("null"), true).unwrap(), None);
    assert_eq!(map_stop_sequences(&json("5"), true).unwrap(), None);
    assert_eq!(map_stop_sequences(&json("true"), true).unwrap(), None);
    assert_eq!(
        map_stop_sequences(&json(r#"{"a": 1}"#), true).unwrap(),
        None
    );
    map_stop_sequences(&json("[1]"), true).unwrap_err();
    map_stop_sequences(&json(r#"["END", null]"#), false).unwrap_err();
}

fn web_search_context_size_case() {
    assert_web_search_location();
    assert_web_search_context();
}

// LiteLLM hands back a plain dict, so the projection is compared as JSON like the oracle sees it
fn web_search(source: &str) -> Json {
    let options: WebSearchOptions = serde_json::from_str(source).unwrap();
    let projected = map_web_search_tool(&options).unwrap();
    serde_json::from_value(serde_json::to_value(projected).unwrap()).unwrap()
}

fn assert_web_search_location() {
    let bare = json(r#"{"type": "web_search_20250305", "name": "web_search"}"#);
    assert_eq!(web_search("{}"), bare);
    assert_eq!(web_search(r#"{"user_location": null}"#), bare);
    assert_eq!(web_search(r#"{"user_location": {}}"#), bare);
    assert_eq!(
        web_search(r#"{"user_location": {"approximate": {}}}"#),
        json(
            r#"{"type": "web_search_20250305", "name": "web_search", "user_location": {"type": "approximate"}}"#
        )
    );
    assert_eq!(
        web_search(
            r#"{"user_location": {"approximate": {"city": "Paris", "country": "FR", "region": "IDF", "timezone": "Europe/Paris"}}}"#
        ),
        json(
            r#"{"type": "web_search_20250305", "name": "web_search", "user_location": {"type": "approximate", "city": "Paris", "country": "FR", "region": "IDF", "timezone": "Europe/Paris"}}"#
        )
    );
    assert_eq!(
        web_search(
            r#"{"user_location": {"approximate": {"type": "elsewhere", "city": "Berlin", "country": "DE", "unknown": 1}}}"#
        ),
        json(
            r#"{"type": "web_search_20250305", "name": "web_search", "user_location": {"type": "approximate", "city": "Berlin", "country": "DE"}}"#
        )
    );
}

fn assert_web_search_context() {
    assert_eq!(
        web_search(r#"{"search_context_size": "low"}"#),
        json(r#"{"type": "web_search_20250305", "name": "web_search", "max_uses": 1}"#)
    );
    assert_eq!(
        web_search(r#"{"search_context_size": "medium"}"#),
        json(r#"{"type": "web_search_20250305", "name": "web_search", "max_uses": 5}"#)
    );
    assert_eq!(
        web_search(r#"{"search_context_size": "high"}"#),
        json(r#"{"type": "web_search_20250305", "name": "web_search", "max_uses": 10}"#)
    );
    assert_eq!(
        web_search(r#"{"search_context_size": null}"#),
        json(r#"{"type": "web_search_20250305", "name": "web_search"}"#)
    );
    let options: WebSearchOptions =
        serde_json::from_str(r#"{"search_context_size": "extreme"}"#).unwrap();
    map_web_search_tool(&options).unwrap_err();
}

fn context_compaction_edit_case() {
    assert_context_passthrough();
    assert_context_compaction();
}

fn context(source: &str) -> Json {
    let mapped = map_context_management(&json(source)).unwrap();
    serde_json::from_value(serde_json::to_value(mapped).unwrap()).unwrap()
}

fn assert_context_passthrough() {
    assert_eq!(
        context(r#"{"edits": [{"type": "compact_20260112"}], "other": [1, 2]}"#),
        json(r#"{"edits": [{"type": "compact_20260112"}], "other": [1, 2]}"#)
    );
    assert_eq!(
        context(r#"{"edits": null, "trigger": true}"#),
        json(r#"{"edits": null, "trigger": true}"#)
    );
    assert_eq!(context(r#"{"type": "compaction"}"#), Json::Null);
    assert_eq!(context("[]"), Json::Null);
    assert_eq!(context(r#""compaction""#), Json::Null);
    assert_eq!(context("null"), Json::Null);
}

fn assert_context_compaction() {
    assert_context_thresholds();
    assert_context_overrides();
}

fn assert_context_thresholds() {
    for (threshold, expected) in [
        ("200000", 200_000_i64),
        ("200000.9", 200_000_i64),
        ("true", 1_i64),
        ("false", 0_i64),
    ] {
        assert_eq!(
            context(&format!(
                r#"[{{"type": "compaction", "compact_threshold": {threshold}}}]"#
            )),
            json(&format!(
                r#"{{"edits": [{{"type": "compact_20260112", "trigger": {{"type": "input_tokens", "value": {expected}}}}}]}}"#
            ))
        );
    }
}

fn assert_context_overrides() {
    let cases = [
        (
            r#"[{"type": "compaction", "compact_threshold": 5, "trigger": {"type": "custom"}, "note": "kept"}]"#,
            r#"{"edits": [{"type": "compact_20260112", "trigger": {"type": "custom"}, "note": "kept"}]}"#,
        ),
        (
            r#"[{"type": "compaction", "compact_threshold": "100"}]"#,
            r#"{"edits": [{"type": "compact_20260112"}]}"#,
        ),
        (
            r#"[5, "x", null, {"type": "other", "compact_threshold": 1}, {"type": "compaction"}]"#,
            r#"{"edits": [{"type": "compact_20260112"}]}"#,
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(context(input), json(expected));
    }
    let huge = json(r#"[{"type": "compaction", "compact_threshold": 1e30}]"#);
    map_context_management(&huge).unwrap_err();
}

fn user_identifier_filter_case() {
    let policy = UserIdPolicy::compile().unwrap();
    let accepted = [
        "user-123",
        "",
        "user@example",
        "user@example.c",
        "\u{fc}@example.com",
        "123456",
        "user@example.com\n\n",
        "user@example.com\r\n",
        "abc def",
        "123 456",
    ];
    let rejected = [
        "user@example.com",
        "USER@EXAMPLE.CO",
        "user.name+tag@sub.example.co.uk",
        "+1 555 010 0000",
        "1234567",
        "123-4567",
        "\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}\u{a0}",
        "\u{661}\u{662}\u{663}\u{664}\u{665}\u{666}\u{667}",
        "\u{1c}1234567",
        " 1234567 ",
        "user@example.com\n",
        "1234567\n",
        "+1234567",
    ];
    for user_id in accepted {
        assert!(policy.accepts(user_id), "{user_id:?} should be accepted");
    }
    for user_id in rejected {
        assert!(!policy.accepts(user_id), "{user_id:?} should be rejected");
    }
}
