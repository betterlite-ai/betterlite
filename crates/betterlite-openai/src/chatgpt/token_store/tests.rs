#![cfg(test)]

use super::codec::{decode_auth_record, decode_token_claims};

#[test]
fn bounds_and_secrets() {
    let record = decode_auth_record(br#"{"access_token":"private-access","refresh_token":"private-refresh","id_token":"private-id","extra":{"password":"private-extra"}}"#).unwrap();
    let debug = format!("{record:?}");
    assert!(!debug.contains("private-"));
    assert_eq!(debug, "AuthRecord { .. }");
    decode_auth_record(b"[]").unwrap_err();
    let bounded = format!("{{}}{}", " ".repeat(super::codec::RECORD_LIMIT - 2));
    decode_auth_record(bounded.as_bytes()).unwrap();
    assert!(matches!(
        decode_auth_record(format!("{bounded} ").as_bytes()),
        Err(super::codec::DecodeError::Size)
    ));
    let boundary = format!("{}.e30=.s", "h".repeat(0x4000 - 7));
    assert!(decode_token_claims(boundary.as_bytes()).is_some());
    let oversized = format!("{}.e30.s", "h".repeat(0x4000));
    assert!(decode_token_claims(oversized.as_bytes()).is_none());
    assert!(decode_token_claims(b"broken").is_none());
}

use rust_decimal::Decimal;
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{fs, io};
use tracing::subscriber::with_default;

use betterlite_config::sections::chatgpt_token_store::Settings;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::codec::AuthRecord;
use super::files::{FileAccess, LocalFiles, StoreEvent};
use super::member::Member;
use super::{ChatGptTokenStore, Clock};

#[derive(Default)]
struct Files {
    records: VecDeque<Vec<u8>>,
    writes: Vec<Vec<u8>>,
    events: Vec<StoreEvent>,
    fail: bool,
}
impl FileAccess for Files {
    fn ensure_dir(&mut self, _path: &Path) -> io::Result<()> {
        Ok(())
    }
    fn read(&mut self, _path: &Path) -> io::Result<Vec<u8>> {
        self.records
            .pop_front()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
    fn write(&mut self, _path: &Path, record: &AuthRecord) -> io::Result<()> {
        self.writes.push(serde_json::to_vec(record)?);
        if self.fail {
            Err(io::Error::from(io::ErrorKind::NotFound))
        } else {
            Ok(())
        }
    }
    fn report(&mut self, event: StoreEvent) {
        self.events.push(event);
    }
}
#[derive(Default)]
struct Time {
    now: Decimal,
    sleeps: Vec<Duration>,
}
impl Clock for Time {
    fn now(&self) -> Decimal {
        self.now
    }
    fn sleep(&mut self, duration: Duration) {
        self.sleeps.push(duration);
        let seconds = Decimal::from(duration.as_secs())
            .checked_add(Decimal::new(i64::from(duration.subsec_nanos()), 9))
            .unwrap();
        self.now = self.now.checked_add(seconds).unwrap();
    }
}
fn store(now: i64) -> ChatGptTokenStore<Files, Time> {
    ChatGptTokenStore::new(
        Settings {
            token_dir: "tokens".to_owned(),
            auth_file: "/auth".to_owned(),
            api_base: "https://configured".to_owned(),
        },
        Files::default(),
        Time {
            now: Decimal::from(now),
            sleeps: Vec::new(),
        },
    )
    .unwrap()
}

#[rstest]
#[case(939, "1000", false)]
#[case(940, "1000", true)]
#[case(941, "1000", true)]
#[case::underflow(1000, "1e-29", true)]
#[case::negative_underflow(1000, "-1e-29", true)]
#[case::positive_overflow(1000, "1e100", true)]
#[case::negative_overflow(1000, "-1e100", true)]
#[case::uncomparable(1000, "1e999", true)]
#[case::underflow_before_epoch(-100, "1e-29", true)]
#[case::plain_underflow(-100, "0.00000000000000000000000000001", true)]
#[case::skew_underflow(0, "-79228162514264337593543950335", true)]
fn expiry_boundary(#[case] now: i64, #[case] expiry: &str, #[case] expired: bool) {
    let mut subject = store(now);
    let mut record =
        decode_auth_record(format!(r#"{{"expires_at":{expiry}}}"#).as_bytes()).unwrap();
    assert_eq!(
        subject.is_expired(&mut record, &SecretString::from("opaque")),
        expired
    );
    assert!(subject.files.writes.is_empty());
    assert_eq!(subject.auth_file, Path::new("/auth"));
    assert_eq!(subject.settings.api_base, "https://configured");
}

#[test]
fn jwt_hints_persist_and_keep_members() {
    let mut subject = store(1);
    let mut record =
        decode_auth_record(br#"{"access_token":"h.eyJleHAiOjEwMDAuOX0.s","extra":{"a":1}}"#)
            .unwrap();
    let token = record
        .access_token
        .as_ref()
        .and_then(Member::known)
        .cloned()
        .unwrap();
    assert!(!subject.is_expired(&mut record, &token));
    let written = decode_auth_record(&subject.files.writes[0]).unwrap();
    assert!(matches!(
        written.expires_at,
        Some(Member::Known(value)) if value.as_i64() == Some(1000)
    ));
    assert_eq!(written.additional_fields, record.additional_fields);
    assert_eq!(
        written
            .access_token
            .as_ref()
            .and_then(Member::known)
            .unwrap()
            .expose_secret(),
        token.expose_secret()
    );
}

#[rstest]
#[case("h.eyJleHAiOjB9.s", true, 0)]
#[case("h.e30.s", true, 0)]
#[case("invalid", true, 0)]
#[case("h.eyJleHAiOjEwMDB9.s", false, 1)]
fn derived_expiry(#[case] token: &str, #[case] expired: bool, #[case] writes: usize) {
    let mut subject = store(1);
    let mut record = decode_auth_record(br#"{"expires_at":"ignored"}"#).unwrap();
    assert_eq!(
        subject.is_expired(&mut record, &SecretString::from(token)),
        expired
    );
    assert_eq!(subject.files.writes.len(), writes);
}

#[rstest]
#[case("100", 200)]
#[case("\"100\"", 200)]
#[case("\"bad\"", 0)]
#[case::unrepresentable_clock("\"1e100\"", 0)]
#[case::nonfinite_clock("\"inf\"", 0)]
#[case("-100", 0)]
#[case("300", 400)]
fn cooldown_clamps(#[case] timestamp: &str, #[case] expected: i64) {
    let subject = store(200);
    let record =
        decode_auth_record(format!("{{\"device_code_requested_at\":{timestamp}}}").as_bytes())
            .unwrap();
    assert_eq!(
        subject.cooldown_remaining(Some(&record)),
        Decimal::from(expected)
    );
    assert_eq!(subject.cooldown_remaining(None), Decimal::ZERO);
}

#[test]
fn failures_and_polling() {
    let mut subject = store(10);
    subject.files.records.push_back(b"[]".to_vec());
    assert!(subject.read().is_none());
    assert_eq!(subject.files.events, [StoreEvent::InvalidRecord]);
    subject.files.fail = true;
    subject.record_device_request();
    assert_eq!(subject.files.events.last(), Some(&StoreEvent::WriteFailure));
    assert!(matches!(
        decode_auth_record(&subject.files.writes[0])
            .unwrap()
            .device_code_requested_at,
        Some(Member::Known(value)) if value.as_i64() == Some(10)
    ));
    subject.files.records.extend([
        b"{}".to_vec(),
        br#"{"access_token":"fresh","expires_at":500}"#.to_vec(),
    ]);
    let token = subject
        .wait_for_access_token(Duration::from_secs(12))
        .unwrap();
    assert_eq!(token.expose_secret(), "fresh");
    assert_eq!(subject.clock.sleeps, [Duration::from_secs(5)]);
    subject.files.records.extend([
        b"{}".to_vec(),
        b"{}".to_vec(),
        br#"{"access_token":"too-late","expires_at":500}"#.to_vec(),
    ]);
    assert!(
        subject
            .wait_for_access_token(Duration::from_secs(7))
            .is_none()
    );
    assert_eq!(
        subject.clock.sleeps,
        [
            Duration::from_secs(5),
            Duration::from_secs(5),
            Duration::from_secs(2)
        ]
    );
}

#[rstest]
#[case::zero(Duration::ZERO, &[], "10")]
#[case::nanosecond(Duration::from_nanos(1), &[Duration::from_nanos(1)], "10.000000001")]
#[case::fraction(Duration::from_millis(1250), &[Duration::from_millis(1250)], "11.25")]
#[case::tail(Duration::new(5, 1), &[Duration::from_secs(5), Duration::from_nanos(1)], "15.000000001")]
fn polling_preserves_nanoseconds(
    #[case] timeout: Duration,
    #[case] sleeps: &[Duration],
    #[case] now: &str,
) {
    let mut subject = store(10);
    assert!(subject.wait_for_access_token(timeout).is_none());
    assert_eq!(subject.clock.sleeps, sleeps);
    assert_eq!(subject.clock.now, now.parse::<Decimal>().unwrap());
}

#[test]
fn local_files_truncate_and_fail() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested");
    LocalFiles.ensure_dir(&path).unwrap();
    let file = path.join("auth.json");
    fs::write(&file, "old contents longer than new").unwrap();
    LocalFiles.write(&file, &AuthRecord::default()).unwrap();
    assert_eq!(fs::read(&file).unwrap(), b"{}");
    LocalFiles.read(&path).unwrap_err();
    assert!(LocalFiles.write(&path, &AuthRecord::default()).is_err());
    assert!(LocalFiles.ensure_dir(&file.join("child")).is_err());
    assert!(
        ChatGptTokenStore::new(
            Settings {
                token_dir: String::new(),
                auth_file: String::new(),
                api_base: String::new()
            },
            LocalFiles,
            Time::default()
        )
        .is_err()
    );
}

#[rstest]
fn replays_auth_record(#[files("fuzz/corpus/auth_record/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _record = decode_auth_record(&bytes);
    let _claims = decode_token_claims(&bytes);
}

#[rstest]
#[case::cached(Some("cached"), Some(r#""h.e30.s""#), Some("cached"), 0)]
#[case::priority(None, Some(r#""h.e30.s""#), None, 0)]
#[case::fallback(None, None, Some("access-account"), 1)]
#[case::empty_id(None, Some(r#""""#), Some("access-account"), 1)]
#[case::null_id(None, Some("null"), Some("access-account"), 1)]
#[case::nonstring_id(None, Some("42"), None, 0)]
fn account_priority(
    #[case] cached: Option<&str>,
    #[case] id: Option<&str>,
    #[case] expected: Option<&str>,
    #[case] writes: usize,
) {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let payload = URL_SAFE_NO_PAD
        .encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"access-account"}}"#);
    let record = AuthRecord {
        access_token: Some(Member::Known(SecretString::from(format!("h.{payload}.s")))),
        id_token: id.map(|value| serde_json::from_str(value).unwrap()),
        account_id: cached.map(str::to_owned).map(Member::Known),
        refresh_token: None,
        expires_at: None,
        device_code_requested_at: None,
        additional_fields: BTreeMap::new(),
    };
    let mut subject = store(0);
    subject
        .files
        .records
        .push_back(serde_json::to_vec(&record).unwrap());
    assert_eq!(subject.get_account_id().as_deref(), expected);
    assert_eq!(subject.files.writes.len(), writes);
}

#[test]
fn build_record_derives_and_drops_unrelated() {
    let record = decode_auth_record(br#"{"access_token":"h.eyJleHAiOjEwMDAuOX0.s","refresh_token":"keep","extra":1,"device_code_requested_at":9}"#).unwrap();
    let built = super::codec::build_auth_record(record);
    assert_eq!(
        serde_json::to_string(&built).unwrap(),
        r#"{"access_token":"h.eyJleHAiOjEwMDAuOX0.s","refresh_token":"keep","id_token":null,"expires_at":1000,"account_id":null}"#
    );
    assert!(built.device_code_requested_at.is_none());
    let nulls = super::codec::build_auth_record(AuthRecord::default());
    assert!(nulls.additional_fields.is_empty());
    assert!(matches!(nulls.account_id, Some(Member::Other(value)) if value.is_null()));
}

#[test]
fn null_members_survive_updates() {
    let mut subject = store(10);
    let record = br#"{"access_token":null,"refresh_token":null,"id_token":null,"expires_at":null,"account_id":null,"device_code_requested_at":null,"extra":null}"#;
    let decoded = decode_auth_record(record).unwrap();
    assert_eq!(decoded.additional_fields.len(), 1);
    subject.files.records.push_back(record.to_vec());
    subject.record_device_request();
    let written = decode_auth_record(&subject.files.writes[0]).unwrap();
    assert_eq!(written.additional_fields.len(), 1);
    assert!(
        matches!(written.device_code_requested_at, Some(Member::Known(value)) if value.as_i64() == Some(10))
    );
}

#[test]
fn optional_strings_ignore_nonstrings_without_warning() {
    let mut subject = store(1);
    let original = br#"{"access_token":42,"id_token":5,"expires_at":{},"extra":[null]}"#;
    subject.files.records.push_back(original.to_vec());
    let mut record = subject.read().unwrap();
    assert!(
        record
            .access_token
            .as_ref()
            .and_then(Member::known)
            .is_none()
    );
    assert!(subject.files.events.is_empty());
    assert!(!subject.is_expired(&mut record, &SecretString::from("h.eyJleHAiOjEwMDB9.s")));
    assert_eq!(record.additional_fields.len(), 1);
    assert!(
        matches!(record.access_token, Some(Member::Other(value)) if value.as_i64() == Some(42))
    );
}

#[test]
fn integer_expiry_preserves_precision() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let payload = URL_SAFE_NO_PAD.encode(br#"{"exp":9007199254740993}"#);
    let token = SecretString::from(format!("h.{payload}.ignored"));
    assert_eq!(
        super::codec::expires_at(&token).unwrap().as_str(),
        "9007199254740993"
    );
}

#[rstest]
#[case::large_fraction("9007199254740993.9", Some("9007199254740993"))]
#[case::negative_fraction("-9007199254740993.9", Some("-9007199254740993"))]
#[case::nearly_one("0.9999999999999999999999999999", Some("0"))]
#[case::scientific("1.009e3", Some("1009"))]
#[case::underflow("1e-29", None)]
#[case::overflow("1e100", None)]
#[case::excess_precision("0.99999999999999999999999999999", None)]
#[case::scientific_precision("0.99999999999999999999999999999e0", None)]
fn fractional_expiry_is_exact(#[case] expiry: &str, #[case] expected: Option<&str>) {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{expiry}}}"#));
    let token = SecretString::from(format!("h.{payload}.s"));
    let actual = super::codec::expires_at(&token);
    assert_eq!(actual.as_ref().map(serde_json::Number::as_str), expected);
}

#[test]
fn default_diagnostics_emit_logs() {
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    with_default(subscriber, || {
        LocalFiles.report(StoreEvent::InvalidRecord);
        LocalFiles.report(StoreEvent::WriteFailure);
    });
    let output = fs::read_to_string(log.path()).unwrap();
    assert_eq!(
        output,
        " WARN Invalid ChatGPT auth file\nERROR Failed to write ChatGPT auth file\n"
    );
}

#[rstest]
#[case::payload_array("[2000000000,null]")]
#[case::auth_array(r#"{"https://api.openai.com/auth":["acct-example"]}"#)]
fn array_claims_provide_no_hints(#[case] claims: &str) {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let payload = URL_SAFE_NO_PAD.encode(claims);
    let token = SecretString::from(format!("h.{payload}.s"));
    let mut record = decode_auth_record(b"{}").unwrap();
    record.access_token = Some(Member::Known(token.clone()));
    let mut subject = store(1000);
    assert!(subject.is_expired(&mut record, &token));
    subject
        .files
        .records
        .push_back(serde_json::to_vec(&record).unwrap());
    assert!(subject.get_account_id().is_none());
    assert!(subject.files.writes.is_empty());
}
