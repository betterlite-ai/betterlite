#![cfg(test)]

use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;

use crate::opencode::ListedModel;

use super::{
    CODEX_BASE_INSTRUCTIONS, CodexCatalogModel, CodexTruncationMode, CodexTruncationPolicy,
    StockCodexCatalog, StockCodexModel, codex_catalog_entry, codex_model_catalog,
};

const INSTRUCTIONS: &str = "fallback instructions";

fn json_value(text: &str) -> JsonValue {
    serde_json::from_str(text).unwrap()
}

fn listed(id: &str, mode: Option<&str>, max_input_tokens: Option<i64>) -> ListedModel {
    ListedModel {
        id: id.to_owned(),
        mode: mode.map(str::to_owned),
        max_input_tokens,
        max_output_tokens: None,
    }
}

fn stock_model(value: JsonValue) -> StockCodexModel {
    serde_json::from_value(value).unwrap()
}

fn served(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

fn catalog_json(models: &[ListedModel], stock: &[StockCodexModel]) -> JsonValue {
    let text = codex_model_catalog(models, stock, INSTRUCTIONS).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn catalog_slugs(models: &[ListedModel], stock: &[StockCodexModel]) -> Vec<String> {
    catalog_json(models, stock)["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["slug"].as_str().unwrap().to_owned())
        .collect()
}

fn gpt_5_4_stock() -> StockCodexModel {
    stock_model(json_value(
        r#"{
            "slug": "gpt-5.4",
            "priority": 16,
            "visibility": "hide",
            "supported_in_api": false,
            "upgrade": {
                "model": "gpt-5.6-terra",
                "migration_markdown": "Switch to GPT-5.6 Terra.",
                "retirement_at": "2026-08-31T19:00:00Z"
            },
            "display_name": "GPT-5.4",
            "description": null,
            "supported_reasoning_levels": [{"effort": "low", "description": "Fast"}],
            "base_instructions": "You are Codex, a coding agent based on GPT-5.",
            "context_window": 272000,
            "comp_hash": "gpt-5.4-hash"
        }"#,
    ))
}

#[test]
fn schema_truncation_policy_defaults_to_bytes_and_ten_thousand() {
    let policy: CodexTruncationPolicy = serde_json::from_value(json_value("{}")).unwrap();
    assert_eq!(policy.mode, CodexTruncationMode::Bytes);
    assert_eq!(policy.limit, 10_000);
    assert_eq!(
        serde_json::to_value(policy).unwrap(),
        json_value(r#"{"mode": "bytes", "limit": 10000}"#),
    );
}

#[test]
fn schema_truncation_policy_accepts_only_the_bytes_mode() {
    serde_json::from_value::<CodexTruncationPolicy>(json_value(r#"{"mode": "tokens"}"#))
        .unwrap_err();
}

#[test]
fn schema_fallback_model_carries_every_codex_field() {
    let entry = codex_catalog_entry(
        7,
        &listed("proxy-model", None, Some(400_000)),
        None,
        &BTreeSet::new(),
        "FALLBACK",
    );
    assert_eq!(
        serde_json::to_value(&entry).unwrap(),
        json_value(
            r#"{
                "slug": "proxy-model",
                "display_name": "proxy-model",
                "description": null,
                "supported_reasoning_levels": [],
                "shell_type": "unified_exec",
                "visibility": "list",
                "supported_in_api": true,
                "priority": 7,
                "availability_nux": null,
                "upgrade": null,
                "support_verbosity": false,
                "supports_reasoning_summaries": false,
                "supports_parallel_tool_calls": false,
                "default_verbosity": null,
                "apply_patch_tool_type": null,
                "truncation_policy": {"mode": "bytes", "limit": 10000},
                "experimental_supported_tools": [],
                "context_window": 400000,
                "base_instructions": "FALLBACK"
            }"#,
        ),
    );
}

#[test]
fn stock_schema_round_trips_every_codex_field() {
    let model = gpt_5_4_stock();
    assert_eq!(
        serde_json::to_value(model).unwrap(),
        json_value(
            r#"{
                "slug": "gpt-5.4",
                "priority": 16,
                "visibility": "hide",
                "supported_in_api": false,
                "upgrade": {
                    "model": "gpt-5.6-terra",
                    "migration_markdown": "Switch to GPT-5.6 Terra.",
                    "retirement_at": "2026-08-31T19:00:00Z"
                },
                "display_name": "GPT-5.4",
                "description": null,
                "supported_reasoning_levels": [{"effort": "low", "description": "Fast"}],
                "base_instructions": "You are Codex, a coding agent based on GPT-5.",
                "context_window": 272000,
                "comp_hash": "gpt-5.4-hash"
            }"#,
        ),
    );
}

#[test]
fn stock_schema_defaults_supported_in_api_and_upgrade() {
    let model = stock_model(json_value(
        r#"{"slug": "gpt-5.5", "priority": 12, "visibility": "list"}"#,
    ));
    assert!(model.supported_in_api);
    assert!(model.upgrade.is_none());
    assert_eq!(
        serde_json::to_value(model).unwrap(),
        json_value(
            r#"{
                "slug": "gpt-5.5",
                "priority": 12,
                "visibility": "list",
                "supported_in_api": true,
                "upgrade": null
            }"#,
        ),
    );
}

#[rstest]
#[case::slug("slug")]
#[case::priority("priority")]
#[case::visibility("visibility")]
fn stock_schema_requires_the_declared_fields(#[case] missing: &str) {
    let mut value = json_value(r#"{"slug": "m", "priority": 1, "visibility": "list"}"#);
    assert!(value.as_object_mut().unwrap().remove(missing).is_some());
    serde_json::from_value::<StockCodexModel>(value).unwrap_err();
}

#[test]
fn stock_schema_accepts_a_negative_priority() {
    let installed = stock_model(json_value(
        r#"{"slug": "m", "priority": -1, "visibility": "list", "display_name": "Stock"}"#,
    ));
    assert_eq!(installed.priority, -1);
    let catalog = catalog_json(&[listed("m", None, None)], &[installed]);
    assert_eq!(catalog["models"][0]["display_name"], "Stock");
    assert_eq!(catalog["models"][0]["priority"].as_u64(), Some(0));
}

#[test]
fn stock_schema_catalog_container_reads_models() {
    let catalog: StockCodexCatalog = serde_json::from_value(json_value(
        r#"{
            "models": [
                {"slug": "a", "priority": 1, "visibility": "list"},
                {"slug": "b", "priority": 2, "visibility": "hide", "future_field": {"nested": true}}
            ]
        }"#,
    ))
    .unwrap();
    assert_eq!(catalog.models.len(), 2);
    assert_eq!(
        catalog.models[1].extra["future_field"],
        json_value(r#"{"nested": true}"#),
    );
}

#[test]
fn unknown_model_entry_copies_the_listing_and_the_instructions() {
    let entry = codex_catalog_entry(
        2,
        &listed("mystery", Some("chat"), Some(128_000)),
        None,
        &served(&["mystery"]),
        "SYSTEM PROMPT",
    );
    let CodexCatalogModel::Fallback(model) = entry else {
        panic!("a model the installed Codex does not know gets the fallback entry")
    };
    assert_eq!(model.slug, "mystery");
    assert_eq!(model.display_name, "mystery");
    assert_eq!(model.priority, 2);
    assert_eq!(model.context_window, Some(128_000));
    assert_eq!(model.base_instructions, "SYSTEM PROMPT");
}

#[test]
fn unknown_model_entry_without_an_input_limit_has_no_context_window() {
    let entry = codex_catalog_entry(
        0,
        &listed("mystery", None, None),
        None,
        &BTreeSet::new(),
        INSTRUCTIONS,
    );
    let CodexCatalogModel::Fallback(model) = entry else {
        panic!("a model the installed Codex does not know gets the fallback entry")
    };
    assert!(model.context_window.is_none());
}

#[test]
fn unknown_model_entry_uses_the_bundled_codex_instructions() {
    let entry = codex_catalog_entry(
        0,
        &listed("mystery", None, None),
        None,
        &BTreeSet::new(),
        CODEX_BASE_INSTRUCTIONS,
    );
    let CodexCatalogModel::Fallback(model) = entry else {
        panic!("a model the installed Codex does not know gets the fallback entry")
    };
    assert_eq!(model.base_instructions, CODEX_BASE_INSTRUCTIONS);
}

#[test]
fn bundled_base_instructions_are_codex_fallback_text() {
    assert!(CODEX_BASE_INSTRUCTIONS.starts_with("You are a coding agent running in the Codex CLI"));
    assert!(CODEX_BASE_INSTRUCTIONS.contains("AGENTS.md"));
    assert!(CODEX_BASE_INSTRUCTIONS.len() > 10_000);
}

#[test]
fn known_model_entry_keeps_stock_metadata_and_takes_proxy_decisions() {
    let installed = gpt_5_4_stock();
    let entry = codex_catalog_entry(
        3,
        &listed("gpt-5.4", None, Some(999_999)),
        Some(&installed),
        &served(&["gpt-5.4", "gpt-5.6-terra"]),
        INSTRUCTIONS,
    );
    let CodexCatalogModel::Known(model) = entry else {
        panic!("a slug the installed Codex knows keeps its stock entry")
    };
    assert_eq!(model.priority, 3);
    assert_eq!(model.visibility, "list");
    assert!(model.supported_in_api);
    assert_eq!(
        model.extra["base_instructions"],
        "You are Codex, a coding agent based on GPT-5."
    );
    assert_eq!(model.extra["context_window"].as_i64(), Some(272_000));
    assert_eq!(
        model.extra["display_name"], "GPT-5.4",
        "the stock display name survives even though the listing omits one"
    );
}

#[test]
fn known_model_entry_keeps_the_stock_upgrade_when_its_target_is_served() {
    let installed = gpt_5_4_stock();
    let entry = codex_catalog_entry(
        0,
        &listed("gpt-5.4", None, None),
        Some(&installed),
        &served(&["gpt-5.4", "gpt-5.6-terra"]),
        INSTRUCTIONS,
    );
    assert_eq!(
        serde_json::to_value(entry).unwrap()["upgrade"],
        json_value(
            r#"{
                "model": "gpt-5.6-terra",
                "migration_markdown": "Switch to GPT-5.6 Terra.",
                "retirement_at": "2026-08-31T19:00:00Z"
            }"#,
        ),
    );
}

#[test]
fn known_model_entry_drops_an_upgrade_when_the_stock_entry_has_none() {
    let installed = stock_model(json_value(
        r#"{"slug": "gpt-5.5", "priority": 12, "visibility": "list"}"#,
    ));
    let entry = codex_catalog_entry(
        0,
        &listed("gpt-5.5", None, None),
        Some(&installed),
        &served(&["gpt-5.5"]),
        INSTRUCTIONS,
    );
    assert!(serde_json::to_value(entry).unwrap()["upgrade"].is_null());
}

#[test]
fn upgrade_filter_drops_an_upgrade_to_an_unserved_model() {
    let installed = gpt_5_4_stock();
    let models = [listed("gpt-5.4", None, None)];
    let catalog = catalog_json(&models, &[installed]);
    assert!(catalog["models"][0]["upgrade"].is_null());
}

#[test]
fn upgrade_filter_keeps_an_upgrade_to_a_served_model() {
    let installed = gpt_5_4_stock();
    let models = [
        listed("gpt-5.4", None, None),
        listed("gpt-5.6-terra", Some("chat"), None),
    ];
    let catalog = catalog_json(&models, &[installed]);
    assert_eq!(
        catalog["models"][0]["upgrade"],
        json_value(
            r#"{
                "model": "gpt-5.6-terra",
                "migration_markdown": "Switch to GPT-5.6 Terra.",
                "retirement_at": "2026-08-31T19:00:00Z"
            }"#,
        ),
    );
}

#[test]
fn served_set_excludes_an_upgrade_target_outside_the_chat_catalog() {
    let installed = gpt_5_4_stock();
    let models = [
        listed("gpt-5.4", None, None),
        listed("gpt-5.6-terra", Some("embedding"), None),
    ];
    let catalog = catalog_json(&models, &[installed]);
    assert!(catalog["models"][0]["upgrade"].is_null());
    assert_eq!(catalog_slugs(&models, &[]), ["gpt-5.4"]);
}

#[test]
fn served_set_includes_a_target_the_chat_filter_kept() {
    let installed = gpt_5_4_stock();
    let models = [
        listed("gpt-5.4", None, None),
        listed("gpt-5.6-terra", None, None),
    ];
    let catalog = catalog_json(&models, &[installed]);
    assert_eq!(catalog["models"][0]["upgrade"]["model"], "gpt-5.6-terra");
}

#[test]
fn empty_catalog_returns_none_for_an_empty_listing() {
    assert!(codex_model_catalog(&[], &[], INSTRUCTIONS).is_none());
}

#[test]
fn empty_catalog_returns_none_when_no_model_is_chat() {
    let models = [listed("embed", Some("embedding"), None)];
    assert!(codex_model_catalog(&models, &[], INSTRUCTIONS).is_none());
}

#[rstest]
#[case::unset(None, true)]
#[case::chat(Some("chat"), true)]
#[case::responses(Some("responses"), true)]
#[case::embedding(Some("embedding"), false)]
#[case::image_generation(Some("image_generation"), false)]
#[case::empty(Some(""), false)]
#[case::capitalized(Some("Chat"), false)]
fn chat_filter_keeps_only_chat_and_responses_modes(#[case] mode: Option<&str>, #[case] kept: bool) {
    let models = [
        listed("probe", mode, None),
        listed("other", Some("chat"), None),
    ];
    let expected = if kept {
        vec!["probe", "other"]
    } else {
        vec!["other"]
    };
    assert_eq!(catalog_slugs(&models, &[]), expected);
}

#[test]
fn chat_filter_preserves_the_listing_order() {
    let models = [
        listed("z", None, None),
        listed("a", Some("responses"), None),
        listed("b", Some("chat"), None),
    ];
    assert_eq!(catalog_slugs(&models, &[]), ["z", "a", "b"]);
}

#[test]
fn stock_lookup_uses_the_stock_entry_for_a_known_slug() {
    let installed = stock_model(json_value(
        r#"{
            "slug": "gpt-5.5",
            "priority": 12,
            "visibility": "list",
            "display_name": "GPT-5.5",
            "base_instructions": "You are Codex, a coding agent based on GPT-5."
        }"#,
    ));
    let catalog = catalog_json(&[listed("gpt-5.5", None, None)], &[installed]);
    assert_eq!(catalog["models"][0]["display_name"], "GPT-5.5");
    assert_eq!(catalog["models"][0]["priority"].as_u64(), Some(0));
}

#[test]
fn stock_lookup_falls_back_for_a_slug_the_stock_lacks() {
    let installed = stock_model(json_value(
        r#"{
            "slug": "gpt-5.5",
            "priority": 12,
            "visibility": "list",
            "display_name": "GPT-5.5"
        }"#,
    ));
    let catalog = catalog_json(&[listed("other", None, None)], &[installed]);
    assert_eq!(catalog["models"][0]["display_name"], "other");
    assert_eq!(
        catalog["models"][0]["truncation_policy"],
        json_value(r#"{"mode": "bytes", "limit": 10000}"#),
    );
}

#[test]
fn stock_lookup_matches_the_whole_slug() {
    let installed = stock_model(json_value(
        r#"{
            "slug": "gpt-5",
            "priority": 12,
            "visibility": "list",
            "display_name": "GPT-5"
        }"#,
    ));
    let catalog = catalog_json(&[listed("gpt-5.5", None, None)], &[installed]);
    assert_eq!(catalog["models"][0]["display_name"], "gpt-5.5");
}

#[test]
fn stock_lookup_keeps_the_last_of_two_duplicate_slugs() {
    let first = stock_model(json_value(
        r#"{"slug": "x", "priority": 1, "visibility": "list", "display_name": "first"}"#,
    ));
    let last = stock_model(json_value(
        r#"{"slug": "x", "priority": 2, "visibility": "list", "display_name": "second"}"#,
    ));
    let catalog = catalog_json(&[listed("x", None, None)], &[first, last]);
    assert_eq!(catalog["models"][0]["display_name"], "second");
}

#[test]
fn json_serialize_wraps_the_entries_in_a_models_object() {
    let models = [listed("m", None, None)];
    let text = codex_model_catalog(&models, &[], INSTRUCTIONS).unwrap();
    let catalog: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(catalog.as_object().unwrap().len(), 1);
    assert_eq!(catalog["models"].as_array().unwrap().len(), 1);
}

#[test]
fn json_serialize_keeps_known_and_fallback_entries_side_by_side() {
    let installed = stock_model(json_value(
        r#"{
            "slug": "gpt-5.5",
            "priority": 12,
            "visibility": "list",
            "display_name": "GPT-5.5"
        }"#,
    ));
    let models = [listed("gpt-5.5", None, None), listed("fresh", None, None)];
    let catalog = catalog_json(&models, &[installed]);
    let entries = catalog["models"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["display_name"], "GPT-5.5");
    assert_eq!(entries[1]["display_name"], "fresh");
    assert_eq!(
        entries[1]["truncation_policy"],
        json_value(r#"{"mode": "bytes", "limit": 10000}"#),
    );
}

#[test]
fn priority_enum_follows_the_filtered_listing_order() {
    let models = [
        listed("first", None, None),
        listed("embed", Some("embedding"), None),
        listed("second", Some("chat"), None),
    ];
    let catalog = catalog_json(&models, &[]);
    let priorities: Vec<u64> = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["priority"].as_u64().unwrap())
        .collect();
    assert_eq!(priorities, [0, 1]);
    assert_eq!(catalog_slugs(&models, &[]), ["first", "second"]);
}

#[test]
fn priority_enum_overwrites_the_stock_priority() {
    let installed = stock_model(json_value(
        r#"{"slug": "gpt-5.5", "priority": 43, "visibility": "list"}"#,
    ));
    let catalog = catalog_json(&[listed("gpt-5.5", None, None)], &[installed]);
    assert_eq!(catalog["models"][0]["priority"].as_u64(), Some(0));
}
