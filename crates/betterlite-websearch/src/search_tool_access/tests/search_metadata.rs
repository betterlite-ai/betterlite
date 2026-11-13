use std::sync::Mutex;

use serde_json::value::RawValue;

use crate::search_tool_access::hook_kwargs::SearchCaller;
use crate::search_tool_access::parent_correlation::ParentRequestCorrelation;
use crate::search_tool_access::search_metadata::{
    SearchMetadataSource, SearchMetadataValue, SearchRequestMetadata, build_search_request_metadata,
};

struct Source {
    calls: Mutex<Vec<String>>,
    metadata: SearchRequestMetadata,
    logged_key: Option<String>,
}

impl Source {
    fn new(metadata: SearchRequestMetadata, logged_key: Option<String>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            metadata,
            logged_key,
        }
    }
}

impl SearchMetadataSource for Source {
    fn sanitized_key_metadata(&self, caller: &SearchCaller) -> SearchRequestMetadata {
        self.calls.lock().unwrap().push(format!(
            "sanitized:{}",
            caller.team_id.as_deref().unwrap_or_default()
        ));
        self.metadata.clone()
    }

    fn logged_api_key(&self, caller: &SearchCaller) -> Option<String> {
        self.calls.lock().unwrap().push(format!(
            "logged:{}",
            caller.team_id.as_deref().unwrap_or_default()
        ));
        self.logged_key.clone()
    }
}

fn raw(text: &str) -> Box<RawValue> {
    serde_json::from_str(text).unwrap()
}

fn caller(team_id: &str) -> SearchCaller {
    serde_json::from_str(&format!(r#"{{"team_id": "{team_id}"}}"#)).unwrap()
}

fn correlation(
    session_id: Option<&str>,
    parent_request_id: Option<&str>,
) -> ParentRequestCorrelation {
    ParentRequestCorrelation {
        session_id: session_id.map(str::to_owned),
        trace_id: None,
        parent_request_id: parent_request_id.map(str::to_owned),
        parent_otel_span: None,
    }
}

fn sanitized() -> SearchRequestMetadata {
    let mut metadata = SearchRequestMetadata::new();
    metadata.insert(
        "user_api_key_alias".to_owned(),
        SearchMetadataValue::Text("alias".to_owned()),
    );
    metadata.insert(
        "user_api_key_team_id".to_owned(),
        SearchMetadataValue::Json(raw(r#""team""#)),
    );
    metadata
}

#[test]
fn merges_key_metadata_correlation_and_identity_in_order() {
    let source = Source::new(sanitized(), Some("sk-logged".to_owned()));
    let metadata = build_search_request_metadata(
        &source,
        &caller("t"),
        Some("search"),
        &correlation(Some("session"), Some("parent")),
    );
    assert_eq!(
        metadata.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "user_api_key_alias",
            "user_api_key_team_id",
            "session_id",
            "parent_request_id",
            "model_group",
            "user_api_key",
            "user_api_key_auth"
        ]
    );
    assert!(matches!(
        &metadata["user_api_key_team_id"],
        SearchMetadataValue::Json(value) if value.get() == "\"team\""
    ));
    assert!(matches!(
        &metadata["session_id"],
        SearchMetadataValue::Text(text) if text == "session"
    ));
    assert!(matches!(
        &metadata["parent_request_id"],
        SearchMetadataValue::Text(text) if text == "parent"
    ));
}

fn stale_sanitized() -> SearchRequestMetadata {
    let mut metadata = sanitized();
    for key in ["model_group", "user_api_key", "user_api_key_auth"] {
        metadata.insert(
            key.to_owned(),
            SearchMetadataValue::Text("stale".to_owned()),
        );
    }
    metadata
}

#[test]
fn later_entries_override_earlier_ones_in_place() {
    let source = Source::new(stale_sanitized(), Some("sk-logged".to_owned()));
    let metadata = build_search_request_metadata(
        &source,
        &caller("t"),
        Some("search"),
        &correlation(Some("session"), None),
    );
    assert_eq!(
        metadata.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "user_api_key_alias",
            "user_api_key_team_id",
            "model_group",
            "user_api_key",
            "user_api_key_auth",
            "session_id"
        ]
    );
    assert!(matches!(
        &metadata["model_group"],
        SearchMetadataValue::Text(text) if text == "search"
    ));
    assert!(matches!(
        &metadata["user_api_key"],
        SearchMetadataValue::Text(text) if text == "sk-logged"
    ));
    assert!(matches!(
        &metadata["user_api_key_auth"],
        SearchMetadataValue::Caller(found) if found.team_id.as_deref() == Some("t")
    ));
}

#[test]
fn a_missing_tool_name_and_logged_key_stay_as_null_values() {
    let source = Source::new(SearchRequestMetadata::new(), None);
    let metadata =
        build_search_request_metadata(&source, &caller("t"), None, &correlation(None, None));
    assert!(matches!(
        &metadata["model_group"],
        SearchMetadataValue::Null
    ));
    assert!(matches!(
        &metadata["user_api_key"],
        SearchMetadataValue::Null
    ));
    assert_eq!(
        metadata.keys().map(String::as_str).collect::<Vec<_>>(),
        ["model_group", "user_api_key", "user_api_key_auth"]
    );
}

#[test]
fn reads_the_source_for_the_request_caller() {
    let source = Source::new(sanitized(), Some("sk-logged".to_owned()));
    let metadata = build_search_request_metadata(
        &source,
        &caller("team"),
        Some("search"),
        &correlation(None, None),
    );
    assert_eq!(metadata.len(), 5);
    assert_eq!(
        source.calls.lock().unwrap().as_slice(),
        ["sanitized:team", "logged:team"]
    );
}
