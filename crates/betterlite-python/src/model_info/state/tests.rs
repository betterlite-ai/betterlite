#![cfg(test)]

use betterlite_config::loader::startup::process_environment;
use betterlite_models::decisions::Json;
use betterlite_models::fetch_policy::revision::git_blob_id;
use betterlite_models::source::{CatalogProvenance, SourceKind};
use chrono::{SecondsFormat, Utc};
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use serde_json::value::RawValue;

use super::{Runtime, catalog_settings};
use crate::model_info::convert::{self, RuntimeEntries};
use crate::model_info::error::ModelInfoError;
use crate::model_info::fetch::header_pairs;
use crate::model_info::git_blob_id as native_git_blob_id;
use crate::model_info::map::finalize;
use crate::native_api::bundled_model_cost_json;

fn runtime() -> Runtime {
    let text = bundled_model_cost_json().unwrap();
    Runtime::new(&text).unwrap()
}

fn fields(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).unwrap()
}

#[test]
fn model_info_state_reports_the_bundled_source() {
    let text = bundled_model_cost_json().unwrap();
    let runtime = Runtime::new(&text).unwrap();
    let info = fields(&runtime.source_json().unwrap());
    assert_eq!(info.len(), 7);
    assert_eq!(info.get("source"), Some(&Json::Text("local".to_owned())));
    assert_eq!(info.get("url"), Some(&Json::Null));
    assert_eq!(info.get("fallback_reason"), Some(&Json::Null));
    assert_eq!(info.get("etag"), Some(&Json::Null));
    assert_eq!(
        info.get("source_revision"),
        Some(&Json::Text(git_blob_id(text.as_bytes())))
    );
    let forced = catalog_settings(&process_environment()).unwrap().local;
    assert_eq!(info.get("is_env_forced"), Some(&Json::Bool(forced)));
    let Json::Text(loaded_at) = info.get("loaded_at").unwrap() else {
        panic!("loaded_at is not ISO text");
    };
    // Python 3.10's datetime.fromisoformat reads 3 or 6 fractional digits, never Rust's 9
    let (_, fraction) = loaded_at.split_once('.').unwrap();
    assert_eq!(fraction.len(), "123456+00:00".len());
    assert!(loaded_at.ends_with("+00:00"));
    assert_eq!(runtime.state().generation, 0);
}

#[test]
fn model_info_state_reports_a_remote_source() {
    let runtime = runtime();
    let loaded_at = Utc::now();
    runtime.update(|state| {
        state.provenance = CatalogProvenance {
            kind: SourceKind::Remote,
            url: Some("https://example.test/map.json".to_owned()),
            is_env_forced: false,
            fallback_reason: Some("Remote fetch failed: HTTP 500".to_owned()),
            loaded_at: Some(loaded_at),
            source_revision: Some("abc".to_owned()),
            etag: Some("etag-1".to_owned()),
        };
    });
    let info = fields(&runtime.source_json().unwrap());
    assert_eq!(info.get("source"), Some(&Json::Text("remote".to_owned())));
    assert_eq!(
        info.get("url"),
        Some(&Json::Text("https://example.test/map.json".to_owned()))
    );
    assert_eq!(info.get("is_env_forced"), Some(&Json::Bool(false)));
    assert_eq!(
        info.get("fallback_reason"),
        Some(&Json::Text("Remote fetch failed: HTTP 500".to_owned()))
    );
    assert_eq!(
        info.get("source_revision"),
        Some(&Json::Text("abc".to_owned()))
    );
    assert_eq!(info.get("etag"), Some(&Json::Text("etag-1".to_owned())));
    assert_eq!(
        info.get("loaded_at"),
        Some(&Json::Text(
            loaded_at.to_rfc3339_opts(SecondsFormat::Micros, false)
        ))
    );
}

#[test]
fn model_info_state_applies_changed_entries_and_removals() {
    let runtime = runtime();
    let request: RuntimeEntries = convert::parse(
        r#"{"entries":{"gpt-4":{"litellm_provider":"openai","max_tokens":1}},"removed":["gpt-4o"]}"#,
    )
    .unwrap();
    runtime.apply_runtime_entries(request);
    let state = runtime.state();
    assert_eq!(
        state.entries.get("gpt-4").map(|value| value.get()),
        Some(r#"{"litellm_provider":"openai","max_tokens":1}"#)
    );
    assert!(!state.entries.contains_key("gpt-4o"));
    assert!(state.entries.contains_key("gpt-4o-mini"));
}

#[test]
fn model_info_state_adopt_of_an_empty_map_returns_zero() {
    let runtime = runtime();
    assert_eq!(runtime.adopt("{}").unwrap(), 0);
    assert!(runtime.state().entries.is_empty());
    assert_eq!(runtime.state().generation, 1);
}

#[test]
fn model_info_state_adopt_counts_the_given_entries() {
    let runtime = runtime();
    let count = runtime
        .adopt(r#"{"acme-a":{"litellm_provider":"acme"},"acme-b":{"litellm_provider":"acme"}}"#)
        .unwrap();
    assert_eq!(count, 2);
    assert_eq!(runtime.state().entries.len(), 2);
    assert_eq!(
        runtime
            .state()
            .entries
            .get("acme-a")
            .map(|value| value.get()),
        Some(r#"{"litellm_provider":"acme"}"#)
    );
}

#[test]
fn model_info_state_adopt_moves_the_generation_every_time() {
    let runtime = runtime();
    assert_eq!(runtime.adopt("{}").unwrap(), 0);
    assert_eq!(runtime.adopt("{}").unwrap(), 0);
    assert_eq!(runtime.state().generation, 2);
}

#[test]
fn model_info_state_adopt_rejects_malformed_text() {
    let runtime = runtime();
    let error = runtime.adopt("{not json").unwrap_err();
    assert!(matches!(error, ModelInfoError::Request(_)));
    assert_eq!(runtime.state().generation, 0);
}

#[test]
fn model_info_state_finalize_installs_rules_and_expands_aliases() {
    let entries: IndexMap<String, Box<RawValue>> = serde_json::from_str(
        r#"{"a":{"litellm_provider":"openai","aliases":["b","d"]},"b":{"litellm_provider":"other"},"c":{"litellm_provider":"openai","aliases":["d"]},"fallback_generalizations":{"rules":[{"name":"acme-ids","pattern":"^acme-","model_info":{"litellm_provider":"acme"}}]}}"#,
    )
    .unwrap();
    let finalized = finalize(entries).unwrap();
    assert_eq!(finalized.rules.len(), 1);
    assert!(!finalized.entries.contains_key("fallback_generalizations"));
    assert_eq!(
        finalized.entries.get("a").map(|value| value.get()),
        Some(r#"{"litellm_provider":"openai"}"#)
    );
    assert_eq!(
        finalized.entries.get("b").map(|value| value.get()),
        Some(r#"{"litellm_provider":"other"}"#)
    );
    assert_eq!(
        finalized.entries.get("d").map(|value| value.get()),
        Some(r#"{"litellm_provider":"openai"}"#)
    );
    assert_eq!(
        finalized.entries.keys().last().map(String::as_str),
        Some("d")
    );
}

#[test]
fn model_info_git_blob_id_hashes_the_git_object_header() {
    assert_eq!(
        native_git_blob_id(b""),
        "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
    );
    assert_eq!(
        native_git_blob_id(b"betterlite"),
        "fa2a1e3b4141ddece01ca48ab95cab199ac377c9"
    );
}

#[test]
fn model_info_fetch_header_pairs_keeps_the_text_headers() {
    let mut headers = HeaderMap::new();
    headers.insert("etag", HeaderValue::from_static("abc"));
    headers.insert("location", HeaderValue::from_static("elsewhere"));
    let mut pairs = header_pairs(&headers);
    pairs.sort();
    assert_eq!(
        pairs,
        vec![
            ("etag".to_owned(), "abc".to_owned()),
            ("location".to_owned(), "elsewhere".to_owned()),
        ]
    );
}

#[test]
fn model_info_state_adopt_rebuilds_the_catalog() {
    let runtime = runtime();
    runtime
        .adopt(r#"{"catalog-probe":{"litellm_provider":"openai","max_tokens":42}}"#)
        .unwrap();
    assert!(runtime.state().catalog.metadata("catalog-probe").is_some());
}
