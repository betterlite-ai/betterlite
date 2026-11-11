#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use rstest::rstest;

use super::{NativeBackend, NativeCacheHandle, NativeCacheLifecycle as _};
use crate::backend::{CacheBackend as _, CacheResult, ConnectionStatus, Ttl};

#[derive(Clone, Debug, PartialEq)]
enum Call {
    Get(String),
    Set(String, CacheValue, Ttl),
    AsyncGet(String),
    AsyncSet(String, CacheValue, Ttl),
    SetMany(Vec<(String, CacheValue)>, Ttl),
    Flush,
    AsyncFlush,
    Ping,
    Disconnect,
    Delete(Vec<String>),
}

#[derive(Default)]
struct HandleState {
    calls: Vec<Call>,
    values: BTreeMap<String, CacheValue>,
    ping: bool,
}

#[derive(Default)]
struct RecordingHandle {
    state: Mutex<HandleState>,
}

impl RecordingHandle {
    fn state(&self) -> MutexGuard<'_, HandleState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl NativeCacheHandle for RecordingHandle {
    fn get(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.state().calls.push(Call::Get(key.to_owned()));
        Ok(self.state().values.get(key).cloned())
    }

    fn set(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.state()
            .calls
            .push(Call::Set(key.to_owned(), value.clone(), ttl));
        self.state().values.insert(key.to_owned(), value);
        Ok(())
    }

    async fn async_get(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.state().calls.push(Call::AsyncGet(key.to_owned()));
        Ok(self.state().values.get(key).cloned())
    }

    async fn async_set(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.state()
            .calls
            .push(Call::AsyncSet(key.to_owned(), value.clone(), ttl));
        self.state().values.insert(key.to_owned(), value);
        Ok(())
    }

    async fn async_set_many(
        &self,
        entries: Vec<(String, CacheValue)>,
        ttl: Ttl,
    ) -> CacheResult<()> {
        self.state().calls.push(Call::SetMany(entries.clone(), ttl));
        for (key, value) in entries {
            self.state().values.insert(key, value);
        }
        Ok(())
    }

    fn flush(&self) -> CacheResult<()> {
        self.state().calls.push(Call::Flush);
        self.state().values.clear();
        Ok(())
    }

    async fn async_flush(&self) -> CacheResult<()> {
        self.state().calls.push(Call::AsyncFlush);
        self.state().values.clear();
        Ok(())
    }

    async fn ping(&self) -> CacheResult<bool> {
        self.state().calls.push(Call::Ping);
        Ok(self.state().ping)
    }

    async fn disconnect(&self) -> CacheResult<()> {
        self.state().calls.push(Call::Disconnect);
        Ok(())
    }

    async fn delete(&self, keys: &[String]) -> CacheResult<()> {
        self.state().calls.push(Call::Delete(keys.to_vec()));
        for key in keys {
            self.state().values.remove(key);
        }
        Ok(())
    }
}

fn backend() -> (NativeBackend, Arc<RecordingHandle>) {
    let handle = Arc::new(RecordingHandle::default());
    let native: Arc<RecordingHandle> = Arc::clone(&handle);
    let backend = NativeBackend { handle: native };
    (backend, handle)
}

fn stored(text: &str) -> CacheValue {
    CacheValue::Text(text.to_owned())
}

#[test]
fn get_cache_returns_the_stored_value_and_records_the_read() {
    let (backend, handle) = backend();
    handle
        .state()
        .values
        .insert("key".to_owned(), stored("cached"));
    assert_eq!(backend.get_cache("key").unwrap(), Some(stored("cached")));
    assert_eq!(handle.state().calls, [Call::Get("key".to_owned())]);
}

#[test]
fn get_cache_returns_none_for_a_missing_key() {
    let (backend, handle) = backend();
    assert_eq!(backend.get_cache("absent").unwrap(), None);
    assert_eq!(handle.state().calls, [Call::Get("absent".to_owned())]);
}

#[rstest]
#[case::absent(None)]
#[case::explicit_zero(Some(0.0_f64))]
#[case::fractional(Some(30.5_f64))]
fn set_cache_forwards_the_value_and_the_ttl(#[case] ttl: Ttl) {
    let (backend, handle) = backend();
    backend.set_cache("key", stored("written"), ttl).unwrap();
    assert_eq!(
        handle.state().calls,
        [Call::Set("key".to_owned(), stored("written"), ttl)]
    );
    assert_eq!(handle.state().values.get("key"), Some(&stored("written")));
}

#[tokio::test]
async fn async_get_cache_forwards_to_the_async_read() {
    let (backend, handle) = backend();
    handle
        .state()
        .values
        .insert("key".to_owned(), stored("cached"));
    assert_eq!(
        backend.async_get_cache("key").await.unwrap(),
        Some(stored("cached"))
    );
    assert_eq!(handle.state().calls, [Call::AsyncGet("key".to_owned())]);
}

#[rstest]
#[case::absent(None)]
#[case::explicit_zero(Some(0.0_f64))]
#[case::fractional(Some(30.5_f64))]
#[tokio::test]
async fn async_set_cache_forwards_the_value_and_the_ttl(#[case] ttl: Ttl) {
    let (backend, handle) = backend();
    backend
        .async_set_cache("key", stored("written"), ttl)
        .await
        .unwrap();
    assert_eq!(
        handle.state().calls,
        [Call::AsyncSet("key".to_owned(), stored("written"), ttl)]
    );
}

#[tokio::test]
async fn async_set_cache_pipeline_forwards_every_entry_in_order() {
    let (backend, handle) = backend();
    let entries = vec![
        ("first".to_owned(), stored("a")),
        ("second".to_owned(), stored("b")),
    ];
    backend
        .async_set_cache_pipeline(entries.clone(), Some(15.0_f64))
        .await
        .unwrap();
    assert_eq!(
        handle.state().calls,
        [Call::SetMany(entries, Some(15.0_f64))]
    );
    assert_eq!(handle.state().values.len(), 2);
}

#[tokio::test]
async fn batch_cache_write_uses_the_single_async_set() {
    let (backend, handle) = backend();
    backend
        .batch_cache_write("key", stored("written"), Some(5.0_f64))
        .await
        .unwrap();
    assert_eq!(
        handle.state().calls,
        [Call::AsyncSet(
            "key".to_owned(),
            stored("written"),
            Some(5.0_f64)
        )]
    );
}

#[test]
fn flush_cache_uses_the_synchronous_flush() {
    let (backend, handle) = backend();
    handle
        .state()
        .values
        .insert("key".to_owned(), stored("cached"));
    backend.flush_cache().unwrap();
    assert_eq!(handle.state().calls, [Call::Flush]);
    assert!(handle.state().values.is_empty());
}

#[tokio::test]
async fn async_flush_cache_uses_the_async_flush() {
    let (backend, handle) = backend();
    handle
        .state()
        .values
        .insert("key".to_owned(), stored("cached"));
    backend.async_flush_cache().await.unwrap();
    assert_eq!(handle.state().calls, [Call::AsyncFlush]);
    assert!(handle.state().values.is_empty());
}

#[rstest]
#[case::reachable(true)]
#[case::unreachable(false)]
#[tokio::test]
async fn ping_returns_the_handles_answer(#[case] answer: bool) {
    let (backend, handle) = backend();
    handle.state().ping = answer;
    assert_eq!(backend.ping().await.unwrap(), answer);
    assert_eq!(handle.state().calls, [Call::Ping]);
}

#[tokio::test]
async fn disconnect_forwards_to_the_handle() {
    let (backend, handle) = backend();
    backend.disconnect().await.unwrap();
    assert_eq!(handle.state().calls, [Call::Disconnect]);
}

#[tokio::test]
async fn delete_cache_keys_forwards_every_key() {
    let (backend, handle) = backend();
    handle
        .state()
        .values
        .insert("first".to_owned(), stored("a"));
    handle
        .state()
        .values
        .insert("second".to_owned(), stored("b"));
    let keys = vec!["first".to_owned(), "second".to_owned()];
    backend.delete_cache_keys(&keys).await.unwrap();
    assert_eq!(handle.state().calls, [Call::Delete(keys)]);
    assert!(handle.state().values.is_empty());
}

#[rstest]
#[case::reachable(true, ConnectionStatus::Success)]
#[case::unreachable(false, ConnectionStatus::Failed)]
#[tokio::test]
async fn test_connection_reports_the_ping_status(
    #[case] answer: bool,
    #[case] expected: ConnectionStatus,
) {
    let (backend, handle) = backend();
    handle.state().ping = answer;
    let test = backend.test_connection().await.unwrap();
    assert_eq!(test.status, expected);
    assert!(!test.message.is_empty());
    assert_eq!(test.error, None);
}

#[test]
fn get_ttl_falls_back_to_the_base_cache_default() {
    let (backend, _handle) = backend();
    assert_eq!(backend.default_ttl(), 60);
    assert_eq!(backend.get_ttl(None), 60);
    assert_eq!(backend.get_ttl(Some(30)), 30);
}
