#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::mcp_toolset_storage::{
    MCPToolset, MCPToolsetStorageError, MCPToolsetTool, ToolsetLookup,
};
use chrono::{DateTime, Utc};
use rstest::rstest;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;

use super::invalidate::invalidate_toolset_cache;
use super::lookup::toolset_by_name_cached;
use super::{CachedToolset, ToolsetByName, ToolsetCache};

const CACHE_TTL_SECS: f64 = 17.5;
const OTHER_TTL_SECS: f64 = 42.25;

fn test_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn test_toolset(name: &str) -> MCPToolset {
    MCPToolset {
        toolset_id: format!("test-id-{name}"),
        toolset_name: name.to_owned(),
        description: Some("test description".to_owned()),
        tools: vec![MCPToolsetTool {
            server_id: "test-server".to_owned(),
            tool_name: "test-tool".to_owned(),
        }],
        created_at: Some(test_time()),
        created_by: Some("test-owner".to_owned()),
        updated_at: Some(test_time()),
        updated_by: Some("test-editor".to_owned()),
    }
}

fn store_failure() -> MCPToolsetStorageError {
    let source = serde_json::from_str::<Vec<MCPToolsetTool>>("not a toolset list").unwrap_err();
    MCPToolsetStorageError::Tools(source)
}

#[derive(Debug, Clone, PartialEq)]
struct CacheWrite {
    key: String,
    value: CachedToolset,
    ttl_secs: f64,
}

#[derive(Debug, thiserror::Error)]
#[error("the fake cache cannot evict entries")]
struct FakeCacheError;

struct FakeCache {
    entries: Mutex<BTreeMap<String, CachedToolset>>,
    writes: Mutex<Vec<CacheWrite>>,
    fail_remove: AtomicBool,
    ttl_secs: f64,
}

impl FakeCache {
    fn new(entries: Vec<(&str, CachedToolset)>, ttl_secs: f64) -> Self {
        let entries = entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        Self {
            entries: Mutex::new(entries),
            writes: Mutex::new(Vec::new()),
            fail_remove: AtomicBool::new(false),
            ttl_secs,
        }
    }

    fn keys(&self) -> Vec<String> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    fn cached(&self, key: &str) -> Option<CachedToolset> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    fn write_log(&self) -> Vec<CacheWrite> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn fail_removals(&self) {
        self.fail_remove.store(true, Ordering::Relaxed);
    }
}

impl ToolsetCache for FakeCache {
    type Error = FakeCacheError;

    fn get(&self, key: &str) -> impl Future<Output = Option<CachedToolset>> + Send {
        ready(self.cached(key))
    }

    fn set(
        &self,
        key: &str,
        value: &CachedToolset,
        ttl_secs: f64,
    ) -> impl Future<Output = ()> + Send {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CacheWrite {
                key: key.to_owned(),
                value: value.clone(),
                ttl_secs,
            });
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value.clone());
        ready(())
    }

    fn remove_where(&self, matches: &dyn Fn(&str) -> bool) -> Result<(), FakeCacheError> {
        if self.fail_remove.load(Ordering::Relaxed) {
            return Err(FakeCacheError);
        }
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|key, _value| !matches(key));
        Ok(())
    }

    fn management_object_ttl(&self) -> f64 {
        self.ttl_secs
    }
}

struct FakeStore {
    toolsets: BTreeMap<String, MCPToolset>,
    calls: Mutex<Vec<String>>,
    fail: bool,
}

impl FakeStore {
    fn new(toolsets: Vec<MCPToolset>) -> Self {
        let toolsets = toolsets
            .into_iter()
            .map(|toolset| (toolset.toolset_name.clone(), toolset))
            .collect();
        Self {
            toolsets,
            calls: Mutex::new(Vec::new()),
            fail: false,
        }
    }

    fn failing() -> Self {
        Self {
            toolsets: BTreeMap::new(),
            calls: Mutex::new(Vec::new()),
            fail: true,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ToolsetByName for FakeStore {
    fn by_name(&self, name: &str) -> impl Future<Output = ToolsetLookup> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
        ready(if self.fail {
            Err(store_failure())
        } else {
            Ok(self.toolsets.get(name).cloned())
        })
    }
}

#[derive(Clone, Default)]
struct RecordingWriter(Arc<Mutex<Vec<u8>>>);

impl RecordingWriter {
    fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8(bytes.clone()).unwrap()
    }
}

impl io::Write for RecordingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture_warnings() -> (RecordingWriter, DefaultGuard) {
    let writer = RecordingWriter::default();
    let sink = writer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || sink.clone())
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .finish();
    (writer, set_default(subscriber))
}

#[tokio::test]
async fn lookup_cached_found_answers_without_reading_the_store() {
    let toolset = test_toolset("test-alpha");
    let cache = FakeCache::new(
        vec![(
            "toolset_name:test-alpha",
            CachedToolset::Found(toolset.clone()),
        )],
        CACHE_TTL_SECS,
    );
    let store = FakeStore::failing();

    let answer = toolset_by_name_cached(&cache, &store, "test-alpha")
        .await
        .unwrap();

    assert_eq!(answer, Some(toolset));
    assert!(store.calls().is_empty());
    assert!(cache.write_log().is_empty());
}

#[tokio::test]
async fn lookup_cached_not_found_answers_without_reading_the_store() {
    let cache = FakeCache::new(
        vec![("toolset_name:test-alpha", CachedToolset::NotFound)],
        CACHE_TTL_SECS,
    );
    let store = FakeStore::new(vec![test_toolset("test-alpha")]);

    let answer = toolset_by_name_cached(&cache, &store, "test-alpha")
        .await
        .unwrap();

    assert_eq!(answer, None);
    assert!(store.calls().is_empty());
    assert!(cache.write_log().is_empty());
}

#[tokio::test]
async fn lookup_miss_reads_the_store_once_and_caches_found() {
    let toolset = test_toolset("test-alpha");
    let cache = FakeCache::new(Vec::new(), CACHE_TTL_SECS);
    let store = FakeStore::new(vec![toolset.clone()]);

    let answer = toolset_by_name_cached(&cache, &store, "test-alpha")
        .await
        .unwrap();

    assert_eq!(answer, Some(toolset.clone()));
    assert_eq!(store.calls(), vec!["test-alpha".to_owned()]);
    assert_eq!(
        cache.cached("toolset_name:test-alpha"),
        Some(CachedToolset::Found(toolset.clone()))
    );
    let writes = cache.write_log();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].key, "toolset_name:test-alpha");
    assert_eq!(writes[0].value, CachedToolset::Found(toolset));
    assert_eq!(writes[0].ttl_secs.to_bits(), CACHE_TTL_SECS.to_bits());
}

#[tokio::test]
async fn lookup_miss_caches_the_not_found_sentinel() {
    let cache = FakeCache::new(Vec::new(), OTHER_TTL_SECS);
    let store = FakeStore::new(Vec::new());

    let answer = toolset_by_name_cached(&cache, &store, "test-missing")
        .await
        .unwrap();

    assert_eq!(answer, None);
    assert_eq!(store.calls(), vec!["test-missing".to_owned()]);
    assert_eq!(
        cache.cached("toolset_name:test-missing"),
        Some(CachedToolset::NotFound)
    );
    let writes = cache.write_log();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].key, "toolset_name:test-missing");
    assert_eq!(writes[0].value, CachedToolset::NotFound);
    assert_eq!(writes[0].ttl_secs.to_bits(), OTHER_TTL_SECS.to_bits());
}

#[tokio::test]
async fn lookup_key_keeps_the_toolset_name_as_given() {
    let name = "test-Alpha Toolset";
    let toolset = test_toolset(name);
    let cache = FakeCache::new(Vec::new(), CACHE_TTL_SECS);
    let store = FakeStore::new(vec![toolset.clone()]);

    toolset_by_name_cached(&cache, &store, name).await.unwrap();

    assert_eq!(
        cache.cached("toolset_name:test-Alpha Toolset"),
        Some(CachedToolset::Found(toolset))
    );
    assert_eq!(store.calls(), vec![name.to_owned()]);
}

#[tokio::test]
async fn lookup_store_error_reaches_the_caller_and_writes_nothing() {
    let cache = FakeCache::new(Vec::new(), CACHE_TTL_SECS);
    let store = FakeStore::failing();

    let answer = toolset_by_name_cached(&cache, &store, "test-alpha").await;

    assert!(answer.is_err());
    assert_eq!(store.calls(), vec!["test-alpha".to_owned()]);
    assert!(cache.write_log().is_empty());
    assert!(cache.keys().is_empty());
}

#[test]
fn invalidate_without_an_id_removes_every_toolset_key() {
    let cache = FakeCache::new(
        vec![
            ("toolset_name:test-alpha", CachedToolset::NotFound),
            ("toolset_perms:test-alpha", CachedToolset::NotFound),
            ("toolset_custom:test-alpha", CachedToolset::NotFound),
            ("toolsetx:test-alpha", CachedToolset::NotFound),
            ("key:test-alpha", CachedToolset::NotFound),
            ("model_list", CachedToolset::NotFound),
        ],
        CACHE_TTL_SECS,
    );

    invalidate_toolset_cache(&cache, None);

    assert_eq!(
        cache.keys(),
        vec![
            "key:test-alpha".to_owned(),
            "model_list".to_owned(),
            "toolsetx:test-alpha".to_owned(),
        ]
    );
}

#[test]
fn invalidate_with_an_id_removes_matching_perms_and_every_name_key() {
    let cache = FakeCache::new(
        vec![
            ("toolset_perms:test-alpha", CachedToolset::NotFound),
            ("toolset_perms:x-test-alpha-y", CachedToolset::NotFound),
            ("toolset_perms:test-alpha-extra", CachedToolset::NotFound),
            ("toolset_perms:test-beta", CachedToolset::NotFound),
            ("toolset_perms_extra:test-alpha", CachedToolset::NotFound),
            ("toolset_name:test-alpha", CachedToolset::NotFound),
            ("toolset_name:test-beta", CachedToolset::NotFound),
            ("toolset_name_extra:test-alpha", CachedToolset::NotFound),
            ("toolset_custom:test-alpha", CachedToolset::NotFound),
            ("toolsetx:test-alpha", CachedToolset::NotFound),
            ("key:test-alpha", CachedToolset::NotFound),
        ],
        CACHE_TTL_SECS,
    );

    invalidate_toolset_cache(&cache, Some("test-alpha"));

    assert_eq!(
        cache.keys(),
        vec![
            "key:test-alpha".to_owned(),
            "toolset_custom:test-alpha".to_owned(),
            "toolset_name_extra:test-alpha".to_owned(),
            "toolset_perms:test-beta".to_owned(),
            "toolset_perms_extra:test-alpha".to_owned(),
            "toolsetx:test-alpha".to_owned(),
        ]
    );
}

#[test]
fn invalidate_logs_a_failed_eviction_and_returns_normally() {
    let cache = FakeCache::new(
        vec![("toolset_name:test-alpha", CachedToolset::NotFound)],
        CACHE_TTL_SECS,
    );
    cache.fail_removals();
    let (captured, _guard) = capture_warnings();

    invalidate_toolset_cache(&cache, None);

    let text = captured.text();
    assert!(text.contains("WARN"));
    assert!(text.contains(
        "invalidate_toolset_cache: failed to evict in-memory entries: the fake cache cannot evict entries"
    ));
    assert_eq!(cache.keys(), vec!["toolset_name:test-alpha".to_owned()]);
}

#[test]
fn wire_found_is_the_toolsets_own_json_object() {
    let toolset = test_toolset("test-alpha");
    let encoded = serde_json::to_value(CachedToolset::Found(toolset.clone())).unwrap();

    assert_eq!(encoded, serde_json::to_value(&toolset).unwrap());
    assert_eq!(encoded["toolset_id"], "test-id-test-alpha");
    assert_eq!(encoded["tools"][0]["tool_name"], "test-tool");
}

#[test]
fn wire_found_decodes_from_the_litellm_json() {
    let text = r#"{
        "toolset_id": "test-id-test-alpha",
        "toolset_name": "test-alpha",
        "description": "test description",
        "tools": [{"server_id": "test-server", "tool_name": "test-tool"}],
        "created_at": "2026-01-02T03:04:05Z",
        "created_by": "test-owner",
        "updated_at": "2026-01-02T03:04:05Z",
        "updated_by": "test-editor"
    }"#;

    let decoded: CachedToolset = serde_json::from_str(text).unwrap();
    let from_value: CachedToolset =
        serde_json::from_value(serde_json::from_str(text).unwrap()).unwrap();

    assert_eq!(decoded, CachedToolset::Found(test_toolset("test-alpha")));
    assert_eq!(from_value, decoded);
}

#[test]
fn wire_found_round_trips_through_json() {
    let cached = CachedToolset::Found(test_toolset("test-alpha"));
    let encoded = serde_json::to_string(&cached).unwrap();
    let decoded: CachedToolset = serde_json::from_str(&encoded).unwrap();
    let from_value: CachedToolset =
        serde_json::from_value(serde_json::to_value(&cached).unwrap()).unwrap();

    assert_eq!(decoded, cached);
    assert_eq!(from_value, cached);
}

#[test]
fn wire_not_found_is_the_bare_sentinel_string() {
    let encoded = serde_json::to_value(CachedToolset::NotFound).unwrap();

    assert_eq!(encoded, "__not_found__");
    assert_eq!(encoded.as_str(), Some("__not_found__"));
}

#[test]
fn wire_not_found_decodes_from_the_sentinel_string() {
    let decoded: CachedToolset = serde_json::from_str("\"__not_found__\"").unwrap();
    let from_value: CachedToolset =
        serde_json::from_value(serde_json::from_str("\"__not_found__\"").unwrap()).unwrap();

    assert_eq!(decoded, CachedToolset::NotFound);
    assert_eq!(from_value, CachedToolset::NotFound);
}

#[test]
fn wire_reports_the_shape_and_the_rejected_value() {
    let wrong_shape = serde_json::from_str::<CachedToolset>("null").unwrap_err();
    let wrong_string = serde_json::from_str::<CachedToolset>(r#""test-other""#).unwrap_err();

    assert!(
        wrong_shape
            .to_string()
            .contains("a toolset object or the __not_found__ string")
    );
    assert!(
        wrong_string
            .to_string()
            .contains(r#"the cache value "test-other" is not a toolset"#)
    );
}

#[rstest]
#[case::other_string(r#""test-other""#)]
#[case::near_sentinel(r#""__not_found__x""#)]
#[case::empty_string(r#""""#)]
#[case::null("null")]
#[case::integer("7")]
#[case::float("1.5")]
#[case::boolean("true")]
#[case::array("[]")]
#[case::empty_object("{}")]
#[case::missing_id(r#"{"toolset_name": "test-alpha"}"#)]
#[case::unknown_field(r#"{"toolset_id": "test-id", "toolset_name": "test-alpha", "extra": true}"#)]
#[case::wrong_field_type(r#"{"toolset_id": 7, "toolset_name": "test-alpha"}"#)]
fn wire_rejects_other_json_values(#[case] text: &str) {
    let from_text = serde_json::from_str::<CachedToolset>(text);
    let value = serde_json::from_str(text).unwrap();
    let from_value = serde_json::from_value::<CachedToolset>(value);

    assert!(from_text.is_err());
    assert!(from_value.is_err());
}
