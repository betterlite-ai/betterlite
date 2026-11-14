#![cfg(test)]

use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use betterlite_logging::private_json::files::PrivateFiles;
use rstest::rstest;
use rustix::process::getuid;
use secrecy::SecretString;
use tempfile::TempDir;
use tokio::runtime::{Builder, Runtime};
use url::ParseError;

use super::{Clock, SessionCache, SystemClock, cache_path, load_session};
use crate::statusline::credentials::Credentials;
use crate::statusline::fetch::error::FetchError;
use crate::statusline::fetch::{Fetched, SessionFetch};
use crate::statusline::session::Session;

const VALID_BODY: &str = r#"{"router_name":"auto","last_model":"gpt-5","spend":1.5}"#;
const START_MILLIS: i64 = 1_700_000_000_123;

struct FakeClock(AtomicI64);

impl Clock for FakeClock {
    fn now_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct FakeFetch {
    calls: AtomicUsize,
    body: Option<&'static str>,
    definitive: bool,
    fails: bool,
}

impl FakeFetch {
    fn answering(body: Option<&'static str>, definitive: bool) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            body,
            definitive,
            fails: false,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl SessionFetch for FakeFetch {
    fn fetch(&self, _credentials: &Credentials, _session_id: &str) -> Result<Fetched, FetchError> {
        let _previous = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fails {
            return Err(FetchError::InvalidUrl(ParseError::EmptyHost));
        }
        Ok(Fetched {
            session: self
                .body
                .and_then(|body| Session::from_json(body.as_bytes())),
            definitive: self.definitive,
        })
    }
}

struct Harness {
    _root: TempDir,
    directory: PathBuf,
    files: PrivateFiles,
    runtime: Runtime,
    clock: FakeClock,
}

fn own_uid() -> u32 {
    getuid().as_raw()
}

fn credentials(base_url: &str, key: &str) -> Credentials {
    Credentials {
        base_url: base_url.to_owned(),
        api_key: SecretString::from(key.to_owned()),
    }
}

fn session() -> Session {
    Session::from_json(VALID_BODY.as_bytes()).unwrap()
}

impl Harness {
    fn new() -> Self {
        let root = TempDir::new().unwrap();
        let directory = root.path().join("tmp").join("litellm-statusline-1000");
        Self {
            _root: root,
            directory,
            files: PrivateFiles::default(),
            runtime: Builder::new_current_thread().enable_all().build().unwrap(),
            clock: FakeClock(AtomicI64::new(START_MILLIS)),
        }
    }

    fn at(&self, millis: i64) {
        self.clock.0.store(millis, Ordering::SeqCst);
    }

    fn load_as(
        &self,
        owner_uid: u32,
        credentials: &Credentials,
        session_id: &str,
        fetch: &FakeFetch,
    ) -> Result<Option<Session>, FetchError> {
        let cache = SessionCache {
            directory: &self.directory,
            owner_uid,
            files: &self.files,
            runtime: &self.runtime,
            clock: &self.clock,
        };
        load_session(&cache, credentials, session_id, fetch)
    }

    fn load(&self, fetch: &FakeFetch) -> Option<Session> {
        self.load_as(own_uid(), &credentials("http://proxy", "key"), "abc", fetch)
            .unwrap()
    }

    fn entry_path(&self) -> PathBuf {
        cache_path(&self.directory, &credentials("http://proxy", "key"), "abc")
    }

    fn private_directory(&self) {
        fs::create_dir_all(&self.directory).unwrap();
        fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn entries(&self) -> Vec<String> {
        fs::read_dir(&self.directory)
            .map(|listing| {
                listing
                    .map(|item| item.unwrap().file_name().to_str().unwrap().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[rstest]
#[case::same_instant(0, 1)]
#[case::just_inside_the_ttl(4_999, 1)]
#[case::exactly_the_ttl(5_000, 2)]
#[case::just_past_the_ttl(5_001, 2)]
#[case::clock_behind_the_entry(-3_000, 1)]
fn an_entry_is_served_until_five_seconds_old(#[case] later_by: i64, #[case] fetches: usize) {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);
    assert_eq!(harness.load(&fetch), Some(session()));

    harness.at(START_MILLIS.saturating_add(later_by));

    assert_eq!(harness.load(&fetch), Some(session()));
    assert_eq!(fetch.calls(), fetches);
}

#[test]
fn a_failed_lookup_is_not_cached() {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(None, false);

    assert_eq!(harness.load(&fetch), None);
    assert_eq!(harness.load(&fetch), None);

    assert_eq!(fetch.calls(), 2);
    assert!(harness.entries().is_empty());
}

#[test]
fn a_settled_answer_without_a_session_is_cached() {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(None, true);

    assert_eq!(harness.load(&fetch), None);
    assert_eq!(harness.load(&fetch), None);

    assert_eq!(fetch.calls(), 1);
}

#[test]
fn a_lookup_that_cannot_run_is_an_error_and_is_not_cached() {
    let harness = Harness::new();
    let fetch = FakeFetch {
        fails: true,
        ..FakeFetch::answering(None, true)
    };

    let failed = harness.load_as(
        own_uid(),
        &credentials("http://proxy", "key"),
        "abc",
        &fetch,
    );

    assert!(matches!(failed, Err(FetchError::InvalidUrl(_))));
    assert!(harness.entries().is_empty());
}

#[rstest]
#[case::another_base_url("http://other", "key", "abc")]
#[case::another_key("http://proxy", "other", "abc")]
#[case::another_session("http://proxy", "key", "other")]
fn an_entry_belongs_to_one_base_url_key_and_session(
    #[case] base_url: &str,
    #[case] key: &str,
    #[case] session_id: &str,
) {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);
    assert_eq!(harness.load(&fetch), Some(session()));

    let other = harness
        .load_as(own_uid(), &credentials(base_url, key), session_id, &fetch)
        .unwrap();

    assert_eq!(other, Some(session()));
    assert_eq!(fetch.calls(), 2);
    assert_eq!(harness.entries().len(), 2);
}

#[rstest]
#[case::all_three(
    "http://proxy",
    "key",
    "abc",
    "7f2e357f36fd5d21f2b2070b25b181a73b19f1753d6ba30cec6c52073a861797"
)]
#[case::base_url(
    "http://proxy2",
    "key",
    "abc",
    "44df4463655c15dbd4118cf0031a2fa64bce321c13e7915f3a705c54f5f42bb1"
)]
#[case::key(
    "http://proxy",
    "key2",
    "abc",
    "c347d2d078493f32e0e8b6206ee809b87c3c9da342cb609a2855bceb49838bfa"
)]
#[case::session(
    "http://proxy",
    "key",
    "abe",
    "d9c152a9c288721535462112e6b0e6d53f935c2262543aa2ea0645411f8414ea"
)]
fn the_entry_name_is_the_sha256_of_the_three_inputs_joined_by_newlines(
    #[case] base_url: &str,
    #[case] key: &str,
    #[case] session_id: &str,
    #[case] expected: &str,
) {
    let path = cache_path(Path::new("/cache"), &credentials(base_url, key), session_id);

    assert_eq!(path, Path::new("/cache").join(expected));
}

#[test]
fn the_directory_is_created_private_with_its_parents() {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);

    let _loaded = harness.load(&fetch);

    let metadata = fs::metadata(&harness.directory).unwrap();
    assert_eq!(metadata.mode() & 0o777, 0o700);
    assert_eq!(harness.entries().len(), 1);
}

#[test]
fn an_entry_holds_the_fetch_time_in_seconds_and_the_session() {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);
    let _loaded = harness.load(&fetch);

    let written = fs::read_to_string(harness.entry_path()).unwrap();

    assert_eq!(
        written,
        r#"{"fetched_at":1700000000.123,"session":{"router_name":"auto","last_model":"gpt-5","spend":1.5,"baseline_spend":null,"baseline_model":null,"turns":null,"savings_estimated_turns":null,"savings_estimated_actual_spend":null}}"#
    );
}

#[test]
fn an_entry_without_a_session_stores_null() {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(None, true);
    let _loaded = harness.load(&fetch);

    let written = fs::read_to_string(harness.entry_path()).unwrap();

    assert_eq!(written, r#"{"fetched_at":1700000000.123,"session":null}"#);
}

#[rstest]
#[case::python_entry(
    r#"{"fetched_at": 1700000001.5, "session": {"router_name": "auto", "last_model": "gpt-5", "spend": 1.5, "baseline_spend": null, "baseline_model": null, "turns": null, "savings_estimated_turns": null, "savings_estimated_actual_spend": null}}"#,
    true,
    0
)]
#[case::exponent_time(
    r#"{"fetched_at": 1.7000000015e9, "session": {"router_name": "auto", "last_model": "gpt-5", "spend": 1.5}}"#,
    true,
    0
)]
#[case::integer_time(
    r#"{"fetched_at": 1700000000, "session": {"router_name": "auto", "last_model": "gpt-5", "spend": 1.5}}"#,
    true,
    0
)]
#[case::null_session(r#"{"fetched_at": 1700000001, "session": null}"#, false, 0)]
#[case::missing_session(r#"{"fetched_at": 1700000001}"#, false, 0)]
#[case::session_that_fails_validation(r#"{"fetched_at": 1700000001, "session": {}}"#, false, 0)]
#[case::session_that_is_not_an_object(r#"{"fetched_at": 1700000001, "session": 7}"#, false, 0)]
#[case::stale_time(
    r#"{"fetched_at": 1699999990, "session": {"router_name": "stale", "last_model": "m", "spend": 1}}"#,
    true,
    1
)]
#[case::string_time(
    r#"{"fetched_at": "1700000001", "session": {"router_name": "stale", "last_model": "m", "spend": 1}}"#,
    true,
    1
)]
#[case::null_time(
    r#"{"fetched_at": null, "session": {"router_name": "stale", "last_model": "m", "spend": 1}}"#,
    true,
    1
)]
#[case::missing_time(
    r#"{"session": {"router_name": "stale", "last_model": "m", "spend": 1}}"#,
    true,
    1
)]
#[case::not_an_object("[1700000001]", true, 1)]
#[case::not_json("nope", true, 1)]
#[case::empty_file("", true, 1)]
fn an_existing_entry_is_read_the_way_litellm_reads_it(
    #[case] content: &str,
    #[case] has_session: bool,
    #[case] fetches: usize,
) {
    let harness = Harness::new();
    harness.private_directory();
    fs::write(harness.entry_path(), content).unwrap();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);

    let served = harness.load(&fetch);

    assert_eq!(fetch.calls(), fetches);
    assert_eq!(served, has_session.then(session));
}

#[rstest]
#[case::group_and_other_readable(0o755)]
#[case::group_readable(0o750)]
#[case::other_readable(0o705)]
#[case::group_writable(0o770)]
fn a_directory_open_to_others_is_neither_read_nor_written(#[case] mode: u32) {
    let harness = Harness::new();
    harness.private_directory();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);
    assert_eq!(harness.load(&fetch), Some(session()));
    fs::set_permissions(&harness.directory, fs::Permissions::from_mode(mode)).unwrap();
    let before = harness.entries();

    assert_eq!(harness.load(&fetch), Some(session()));

    assert_eq!(fetch.calls(), 2);
    assert_eq!(harness.entries(), before);
    assert_eq!(
        fs::metadata(&harness.directory).unwrap().mode() & 0o777,
        mode
    );
}

#[test]
fn a_directory_owned_by_someone_else_is_neither_read_nor_written() {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);
    let credentials = credentials("http://proxy", "key");
    let other_uid = own_uid().saturating_add(1);

    let first = harness
        .load_as(other_uid, &credentials, "abc", &fetch)
        .unwrap();
    let second = harness
        .load_as(other_uid, &credentials, "abc", &fetch)
        .unwrap();

    assert_eq!((first, second), (Some(session()), Some(session())));
    assert_eq!(fetch.calls(), 2);
    assert!(harness.entries().is_empty());
}

#[test]
fn a_directory_that_is_a_link_is_neither_read_nor_written() {
    let harness = Harness::new();
    let real = harness.directory.with_file_name("real");
    fs::create_dir_all(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
    symlink(&real, &harness.directory).unwrap();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);

    let _first = harness.load(&fetch);
    let _second = harness.load(&fetch);

    assert_eq!(fetch.calls(), 2);
    assert!(fs::read_dir(&real).unwrap().next().is_none());
}

#[test]
fn an_entry_that_is_a_link_is_not_followed() {
    let harness = Harness::new();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);
    let _first = harness.load(&fetch);
    let copy = harness.directory.join("copy");
    fs::rename(harness.entry_path(), &copy).unwrap();
    symlink(&copy, harness.entry_path()).unwrap();

    assert_eq!(harness.load(&fetch), Some(session()));

    assert_eq!(fetch.calls(), 2);
}

#[test]
fn a_cache_that_cannot_be_written_does_not_fail_the_lookup() {
    let harness = Harness::new();
    harness.private_directory();
    let blocker = harness.entry_path();
    fs::create_dir_all(&blocker).unwrap();
    fs::write(blocker.join("occupied"), "x").unwrap();
    let fetch = FakeFetch::answering(Some(VALID_BODY), true);

    assert_eq!(harness.load(&fetch), Some(session()));

    assert!(blocker.is_dir());
}

#[test]
fn the_system_clock_reads_the_current_time_in_milliseconds() {
    let now = SystemClock.now_millis();

    assert!(now > 1_700_000_000_000, "{now}");
    assert!(now < 4_000_000_000_000, "{now}");
}
