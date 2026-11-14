#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex};

use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{ScanCacheStore, scan_cache_key};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::settings::GuardrailSettings;

type Body = BTreeMap<String, GuardrailValue>;

const HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
const WORLD: &str = "486ea46224d1bb4fb680f34f7c9ad96a8f24ec88be73ea8e5a6c65260e9cb8a7";
const ACCENTED: &str = "3c48591d8d098a4538f5e013dfcf406e948eac4d3277b10bf614e295d6068179";
const KEY: &str = "guardrail_scanned_texts:g:s1";

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn capture() -> (DefaultGuard, Captured) {
    let captured = Captured::default();
    let sink = captured.clone();
    let subscriber = fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || sink.clone())
        .finish();
    (set_default(subscriber), captured)
}

#[derive(Default)]
struct Store {
    entries: Mutex<BTreeMap<String, Vec<String>>>,
    reads: Mutex<Vec<String>>,
    writes: Mutex<Vec<(String, Vec<String>, i64)>>,
    fail_reads: bool,
    fail_writes: bool,
}

impl Store {
    fn holding(key: &str, hashes: &[&str]) -> Self {
        let store = Self::default();
        store.entries.lock().unwrap().insert(
            key.to_owned(),
            hashes.iter().map(|hash| (*hash).to_owned()).collect(),
        );
        store
    }

    fn failing_reads() -> Self {
        Self {
            entries: Mutex::default(),
            reads: Mutex::default(),
            writes: Mutex::default(),
            fail_reads: true,
            fail_writes: false,
        }
    }

    fn failing_writes() -> Self {
        Self {
            entries: Mutex::default(),
            reads: Mutex::default(),
            writes: Mutex::default(),
            fail_reads: false,
            fail_writes: true,
        }
    }

    fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }

    fn writes(&self) -> Vec<(String, Vec<String>, i64)> {
        self.writes.lock().unwrap().clone()
    }
}

impl ScanCacheStore for Store {
    type Error = io::Error;

    async fn get(&self, key: &str) -> Result<Option<Vec<String>>, io::Error> {
        self.reads.lock().unwrap().push(key.to_owned());
        if self.fail_reads {
            return Err(io::Error::other("boom"));
        }
        Ok(self.entries.lock().unwrap().get(key).cloned())
    }

    async fn set(&self, key: &str, hashes: Vec<String>, ttl_seconds: i64) -> Result<(), io::Error> {
        if self.fail_writes {
            return Err(io::Error::other("boom"));
        }
        self.writes
            .lock()
            .unwrap()
            .push((key.to_owned(), hashes.clone(), ttl_seconds));
        self.entries.lock().unwrap().insert(key.to_owned(), hashes);
        Ok(())
    }
}

fn scanning(name: Option<&str>) -> GuardrailBase {
    let mut base = GuardrailBase {
        settings: GuardrailSettings::default(),
        mode: None,
    };
    base.settings.guardrail_name = name.map(str::to_owned);
    base.settings.only_scan_new_messages = true;
    base
}

fn session(id: &str) -> Body {
    BTreeMap::from([(
        "litellm_session_id".to_owned(),
        GuardrailValue::Text(id.to_owned()),
    )])
}

fn texts(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[rstest]
#[case::named(Some("g"), "guardrail_scanned_texts:g:s1")]
#[case::unnamed(None, "guardrail_scanned_texts:None:s1")]
#[case::empty_name(Some(""), "guardrail_scanned_texts::s1")]
fn filter_key_has_the_name_and_session(#[case] name: Option<&str>, #[case] expected: &str) {
    assert_eq!(scan_cache_key(name, "s1"), expected);
}

#[tokio::test]
async fn filter_returns_only_unseen_texts_in_order_and_keeps_duplicates() {
    let store = Store::holding(KEY, &[WORLD]);
    let result = scanning(Some("g"))
        .filter_new_texts_for_session(
            Some(&texts(&["hello", "world", "hello"])),
            &session("s1"),
            &store,
        )
        .await;
    assert_eq!(result, Some(texts(&["hello", "hello"])));
    assert_eq!(store.reads(), [KEY]);
}

#[tokio::test]
async fn filter_returns_an_empty_list_when_every_text_was_seen() {
    let store = Store::holding(KEY, &[WORLD, HELLO]);
    let result = scanning(Some("g"))
        .filter_new_texts_for_session(Some(&texts(&["hello", "world"])), &session("s1"), &store)
        .await;
    assert_eq!(result, Some(Vec::new()));
}

#[tokio::test]
async fn filter_returns_every_text_when_nothing_is_stored() {
    let store = Store::default();
    let result = scanning(Some("g"))
        .filter_new_texts_for_session(Some(&texts(&["hello", "world"])), &session("s1"), &store)
        .await;
    assert_eq!(result, Some(texts(&["hello", "world"])));
}

#[tokio::test]
async fn filter_hashes_the_utf8_bytes_as_lowercase_hex() {
    let store = Store::holding(KEY, &[ACCENTED]);
    let result = scanning(Some("g"))
        .filter_new_texts_for_session(Some(&texts(&["héllo", "hello"])), &session("s1"), &store)
        .await;
    assert_eq!(result, Some(texts(&["hello"])));
}

#[tokio::test]
async fn filter_reads_the_key_of_an_unnamed_guardrail_with_none() {
    let store = Store::default();
    let result = scanning(None)
        .filter_new_texts_for_session(Some(&texts(&["hello"])), &session("s1"), &store)
        .await;
    assert_eq!(result, Some(texts(&["hello"])));
    assert_eq!(store.reads(), ["guardrail_scanned_texts:None:s1"]);
}

#[tokio::test]
async fn filter_scans_everything_when_the_feature_is_off() {
    let mut base = scanning(Some("g"));
    base.settings.only_scan_new_messages = false;
    let store = Store::holding(KEY, &[HELLO]);
    let (_guard, logs) = capture();
    let result = base
        .filter_new_texts_for_session(Some(&texts(&["hello"])), &session("s1"), &store)
        .await;
    assert_eq!(result, None);
    assert!(store.reads().is_empty());
    assert_eq!(logs.text(), "");
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(Vec::new()))]
#[tokio::test]
async fn filter_scans_everything_without_texts(#[case] given: Option<Vec<String>>) {
    let store = Store::holding(KEY, &[HELLO]);
    let result = scanning(Some("g"))
        .filter_new_texts_for_session(given.as_deref(), &session("s1"), &store)
        .await;
    assert_eq!(result, None);
    assert!(store.reads().is_empty());
}

#[rstest]
#[case::request(true)]
#[case::response(false)]
#[tokio::test]
async fn filter_warns_and_scans_everything_with_masking(#[case] request: bool) {
    let mut base = scanning(Some("g"));
    base.settings.mask_request_content = request;
    base.settings.mask_response_content = !request;
    let store = Store::holding(KEY, &[HELLO]);
    let (_guard, logs) = capture();
    let result = base
        .filter_new_texts_for_session(Some(&texts(&["hello"])), &session("s1"), &store)
        .await;
    assert_eq!(result, None);
    assert!(store.reads().is_empty());
    let output = logs.text();
    assert!(output.contains("WARN"));
    assert!(output.contains(
        "Guardrail g: only_scan_new_messages is not supported with masking; scanning full context."
    ));
}

#[tokio::test]
async fn filter_names_an_unnamed_guardrail_none_in_the_masking_warning() {
    let mut base = scanning(None);
    base.settings.mask_request_content = true;
    let (_guard, logs) = capture();
    let result = base
        .filter_new_texts_for_session(Some(&texts(&["hello"])), &session("s1"), &Store::default())
        .await;
    assert_eq!(result, None);
    assert!(
        logs.text()
            .contains("Guardrail None: only_scan_new_messages is not supported with masking")
    );
}

#[tokio::test]
async fn filter_scans_everything_when_the_request_has_no_session_id() {
    let store = Store::holding(KEY, &[HELLO]);
    let (_guard, logs) = capture();
    let result = scanning(Some("g"))
        .filter_new_texts_for_session(Some(&texts(&["hello"])), &Body::new(), &store)
        .await;
    assert_eq!(result, None);
    assert!(store.reads().is_empty());
    let output = logs.text();
    assert!(output.contains("DEBUG"));
    assert!(output.contains(
        "Guardrail g: only_scan_new_messages enabled but request has no session id; scanning full context."
    ));
}

#[tokio::test]
async fn filter_warns_and_scans_everything_when_the_read_fails() {
    let store = Store::failing_reads();
    let (_guard, logs) = capture();
    let result = scanning(Some("g"))
        .filter_new_texts_for_session(Some(&texts(&["hello"])), &session("s1"), &store)
        .await;
    assert_eq!(result, None);
    let output = logs.text();
    assert!(output.contains("WARN"));
    assert!(output.contains(
        "Guardrail g: failed to read scanned-message cache (boom); scanning full context."
    ));
}

#[rstest]
#[case::default_ttl(86400)]
#[case::zero(0)]
#[case::negative(-5)]
#[tokio::test]
async fn mark_writes_the_hashes_with_the_given_ttl(#[case] ttl: i64) {
    let store = Store::default();
    scanning(Some("g"))
        .mark_texts_scanned(
            Some(&texts(&["hello", "world"])),
            &session("s1"),
            &store,
            ttl,
        )
        .await;
    assert_eq!(
        store.writes(),
        [(KEY.to_owned(), texts(&[HELLO, WORLD]), ttl)]
    );
}

#[tokio::test]
async fn mark_appends_new_hashes_after_the_stored_ones_without_repeats() {
    let store = Store::holding(KEY, &[WORLD, "other"]);
    scanning(Some("g"))
        .mark_texts_scanned(
            Some(&texts(&["hello", "world", "hello"])),
            &session("s1"),
            &store,
            60,
        )
        .await;
    assert_eq!(
        store.writes(),
        [(KEY.to_owned(), texts(&[WORLD, "other", HELLO]), 60)]
    );
}

#[tokio::test]
async fn mark_hashes_the_utf8_bytes_as_lowercase_hex() {
    let store = Store::default();
    scanning(None)
        .mark_texts_scanned(Some(&texts(&["héllo"])), &session("s1"), &store, 60)
        .await;
    assert_eq!(
        store.writes(),
        [(
            "guardrail_scanned_texts:None:s1".to_owned(),
            texts(&[ACCENTED]),
            60
        )]
    );
}

#[tokio::test]
async fn mark_makes_filter_skip_the_marked_texts() {
    let store = Store::default();
    let base = scanning(Some("g"));
    let request = session("s1");
    base.mark_texts_scanned(Some(&texts(&["hello"])), &request, &store, 60)
        .await;
    let result = base
        .filter_new_texts_for_session(Some(&texts(&["hello", "world"])), &request, &store)
        .await;
    assert_eq!(result, Some(texts(&["world"])));
}

#[tokio::test]
async fn mark_does_nothing_when_the_feature_is_off() {
    let mut base = scanning(Some("g"));
    base.settings.only_scan_new_messages = false;
    let store = Store::default();
    let (_guard, logs) = capture();
    base.mark_texts_scanned(Some(&texts(&["hello"])), &session("s1"), &store, 60)
        .await;
    assert!(store.reads().is_empty());
    assert!(store.writes().is_empty());
    assert_eq!(logs.text(), "");
}

#[rstest]
#[case::missing(None)]
#[case::empty(Some(Vec::new()))]
#[tokio::test]
async fn mark_does_nothing_without_texts(#[case] given: Option<Vec<String>>) {
    let store = Store::default();
    scanning(Some("g"))
        .mark_texts_scanned(given.as_deref(), &session("s1"), &store, 60)
        .await;
    assert!(store.reads().is_empty());
    assert!(store.writes().is_empty());
}

#[rstest]
#[case::request(true)]
#[case::response(false)]
#[tokio::test]
async fn mark_does_nothing_with_masking(#[case] request: bool) {
    let mut base = scanning(Some("g"));
    base.settings.mask_request_content = request;
    base.settings.mask_response_content = !request;
    let store = Store::default();
    let (_guard, logs) = capture();
    base.mark_texts_scanned(Some(&texts(&["hello"])), &session("s1"), &store, 60)
        .await;
    assert!(store.reads().is_empty());
    assert!(store.writes().is_empty());
    assert_eq!(logs.text(), "");
}

#[tokio::test]
async fn mark_does_nothing_without_a_session_id() {
    let store = Store::default();
    let (_guard, logs) = capture();
    scanning(Some("g"))
        .mark_texts_scanned(Some(&texts(&["hello"])), &Body::new(), &store, 60)
        .await;
    assert!(store.reads().is_empty());
    assert!(store.writes().is_empty());
    assert_eq!(logs.text(), "");
}

#[tokio::test]
async fn mark_warns_and_writes_nothing_when_the_read_fails() {
    let store = Store::failing_reads();
    let (_guard, logs) = capture();
    scanning(Some("g"))
        .mark_texts_scanned(Some(&texts(&["hello"])), &session("s1"), &store, 60)
        .await;
    assert!(store.writes().is_empty());
    let output = logs.text();
    assert!(output.contains("WARN"));
    assert!(output.contains(
        "Guardrail g: failed to persist scanned-message cache (boom); next call will re-scan."
    ));
}

#[tokio::test]
async fn mark_warns_once_when_the_write_fails() {
    let store = Store::failing_writes();
    let (_guard, logs) = capture();
    scanning(Some("g"))
        .mark_texts_scanned(Some(&texts(&["hello"])), &session("s1"), &store, 60)
        .await;
    assert!(store.writes().is_empty());
    let output = logs.text();
    assert_eq!(
        output
            .matches("failed to persist scanned-message cache (boom)")
            .count(),
        1
    );
    assert!(output.contains("WARN"));
}
