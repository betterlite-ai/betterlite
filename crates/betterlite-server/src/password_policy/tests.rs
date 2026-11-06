#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::read_to_string;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::password_policy::{CharacterClass, Settings};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rstest::rstest;
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::subscriber::{DefaultGuard, set_default};

use super::breach::{BreachChecker, HIBP_RANGE_API_BASE, HIBP_TIMEOUT, LookupError};
use super::bulk::validate_passwords_bulk;
use super::error::{ERROR_TYPE, PARAM, PasswordError, STATUS};
use super::hashing::{HashError, OsSalts, PasswordHasher, SaltSource};
use super::migration::{PasswordStore, StoredPassword, migrate_passwords_to_scrypt};
use super::strength::{validate, violations};

// scrypt:base64(salt || key) computed by CPython's hashlib.scrypt for the password below and salt bytes 0..16.
const PASSWORD: &str = "Tr0ub4dor&3-horse";
const PYTHON_SCRYPT: &str =
    "scrypt:AAECAwQFBgcICQoLDA0ODx1Hv3tNBnic5gedoiP9VrMdNNYQltZrnQ2BXv57yZCs";
const PYTHON_SHA256: &str = "ed959ca4a127cf7607afafe90c900b004d51ae2e40be4d1419114915a1f3db01";

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn defaults() -> Settings {
    Settings::default()
}

fn loaded(yaml: &str) -> Settings {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    loader.load(yaml).unwrap().sections.password_policy
}

struct FixedSalts;

impl SaltSource for FixedSalts {
    fn fill(&self, salt: &mut [u8; 16]) -> Result<(), HashError> {
        for (index, byte) in salt.iter_mut().enumerate() {
            *byte = u8::try_from(index).map_err(|_overflow| HashError::Params)?;
        }
        Ok(())
    }
}

#[tokio::test]
async fn scrypt_compatible_hash() {
    let hasher = PasswordHasher::new(FixedSalts, 2);
    let derived = hasher.hash(&secret(PASSWORD)).await.unwrap();
    assert_eq!(derived, PYTHON_SCRYPT);
    assert!(hasher.verify(&secret(PASSWORD), PYTHON_SCRYPT).await);
    assert!(
        !hasher
            .verify(&secret("Tr0ub4dor&3-horsf"), PYTHON_SCRYPT)
            .await
    );
    assert!(!hasher.verify(&secret(""), PYTHON_SCRYPT).await);

    let random = PasswordHasher::new(OsSalts, 2);
    let first = random.hash(&secret(PASSWORD)).await.unwrap();
    let second = random.hash(&secret(PASSWORD)).await.unwrap();
    assert_ne!(first, second);
    assert!(first.starts_with("scrypt:"));
    assert!(random.verify(&secret(PASSWORD), &first).await);
    assert!(random.verify(&secret(PASSWORD), &second).await);
}

#[rstest]
#[case::not_base64("scrypt:!!!")]
#[case::bad_padding("scrypt:AAEC=")]
#[case::empty_payload("scrypt:")]
#[case::salt_only("scrypt:AAECAwQFBgcICQoLDA0ODw==")]
#[case::truncated_key("scrypt:AAECAwQFBgcICQoLDA0ODx1Hv3tNBnic5gedoiP9")]
#[case::wrong_prefix_case(
    "SCRYPT:AAECAwQFBgcICQoLDA0ODx1Hv3tNBnic5gedoiP9VrMdNNYQltZrnQ2BXv57yZCs"
)]
#[tokio::test]
async fn malformed_scrypt_never_verifies(#[case] stored: &str) {
    let hasher = PasswordHasher::new(FixedSalts, 1);
    assert!(!hasher.verify(&secret(PASSWORD), stored).await);
}

#[tokio::test]
async fn scrypt_payload_ignores_characters_outside_the_alphabet() {
    let hasher = PasswordHasher::new(FixedSalts, 1);
    let (head, tail) = PYTHON_SCRYPT.split_at(30);
    let wrapped = format!("{head}\n{tail}");
    assert!(hasher.verify(&secret(PASSWORD), &wrapped).await);
    let accented = format!("{head}\u{e9}{tail}");
    assert!(!hasher.verify(&secret(PASSWORD), &accented).await);
}

#[tokio::test]
async fn legacy_sha256() {
    let hasher = PasswordHasher::new(FixedSalts, 1);
    assert!(hasher.verify(&secret(PASSWORD), PYTHON_SHA256).await);
    assert!(!hasher.verify(&secret("other"), PYTHON_SHA256).await);
    let upper = PYTHON_SHA256.to_uppercase();
    assert!(!hasher.verify(&secret(PASSWORD), &upper).await);
    let (_first, rest) = PYTHON_SHA256.split_at(1);
    assert!(!hasher.verify(&secret(PASSWORD), rest).await);
    assert!(
        !hasher
            .verify(&secret(PASSWORD), &format!("{PYTHON_SHA256}0"))
            .await
    );
    assert!(!hasher.verify(&secret(PASSWORD), PASSWORD).await);
    assert!(!hasher.verify(&secret(PASSWORD), "").await);
    let not_hex = format!("g{rest}");
    assert!(!hasher.verify(&secret(PASSWORD), &not_hex).await);
}

#[rstest]
#[case::ascii_special("Passw0rd!Passw0rd", &[])]
#[case::accented_letter_is_not_special("Passw\u{f6}rd1234", &["include a special character"])]
#[case::superscript_digit_is_a_digit("Passw\u{f6}rd1\u{b2}34!", &[])]
#[case::lowercase_combining_mark_is_special("Pass\u{345}w\u{f6}rd1234", &[])]
#[case::cyrillic_letters("\u{41f}\u{430}\u{440}\u{43e}\u{43b}\u{44c}1234Abc", &["include a special character"])]
#[case::cyrillic_with_special("\u{41f}\u{430}\u{440}\u{43e}\u{43b}\u{44c}1234Abc!", &[])]
#[case::arabic_indic_digits("\u{391}\u{392}\u{393}\u{3b4}\u{3b5}\u{3b6}\u{663}\u{664}\u{665}\u{666}\u{667}\u{668}\u{669}", &["include a special character"])]
#[case::vulgar_fraction_is_alphanumeric_not_a_digit("Abcdefghij\u{bd}\u{bd}", &["include a number", "include a special character"])]
#[case::titlecase_letters_are_neither_case("\u{1c5}\u{1c5}\u{1c5}\u{1c5}\u{1c5}\u{1c5}\u{1c5}\u{1c5}\u{1c5}\u{1c5}\u{1c5}\u{1c5}", &["include an uppercase letter", "include a lowercase letter", "include a number", "include a special character"])]
#[case::cjk_has_no_case("\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{30d1}\u{30b9}\u{30ef}\u{30fc}\u{30c9}\u{3067}\u{3059}1A", &["include a lowercase letter", "include a special character"])]
#[case::length_counts_code_points("Aa1!\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}", &[])]
#[case::one_code_point_short("Aa1!\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}", &["be at least 12 characters long"])]
#[case::roman_numerals_are_uppercase("\u{2167}\u{2167}\u{2167}\u{2167}\u{2167}\u{2167}\u{2167}\u{2167}1a!", &["be at least 12 characters long"])]
#[case::space_is_special("Abcdefghi1_ ", &[])]
#[case::everything_missing("", &["be at least 12 characters long", "include an uppercase letter", "include a lowercase letter", "include a number", "include a special character"])]
fn unicode_strength(#[case] password: &str, #[case] expected: &[&str]) {
    let found = violations(&secret(password), &defaults());
    assert_eq!(found, expected);
    match validate(&secret(password), &defaults()) {
        Ok(()) => assert!(expected.is_empty()),
        Err(error) => assert_eq!(
            error.to_string(),
            format!(
                "Password does not meet the required policy: must {}.",
                expected.join(", ")
            )
        ),
    }
}

#[test]
fn disabled_classes_are_not_checked() {
    let settings = Settings {
        min_length: 8,
        required: vec![CharacterClass::Number],
        check_breached_passwords: true,
    };
    assert!(validate(&secret("lowercase"), &settings).is_err());
    assert_eq!(validate(&secret("lowercase9"), &settings), Ok(()));
    let none = loaded(
        "general_settings:\n  password_policy_require_uppercase: false\n  password_policy_require_lowercase: false\n  password_policy_require_numbers: false\n  password_policy_require_special_characters: false\n",
    );
    assert_eq!(validate(&secret("abcdefghijkl"), &none), Ok(()));
}

#[rstest]
#[case::below_floor(0, "Ab1!xyz", "be at least 8 characters long")]
#[case::negative(-5, "Ab1!xyz", "be at least 8 characters long")]
#[case::three(3, "Ab1!xyz", "be at least 8 characters long")]
#[case::above_floor(20, "Ab1!xyz-Ab1!xyz-Ab1", "be at least 20 characters long")]
fn minimum_floor(#[case] configured: i64, #[case] password: &str, #[case] message: &str) {
    let settings = loaded(&format!(
        "general_settings:\n  password_policy_min_length: {configured}\n"
    ));
    let error = validate(&secret(password), &settings).unwrap_err();
    assert_eq!(
        error,
        PasswordError::Policy {
            requirements: message.to_owned()
        }
    );
    assert!(validate(&secret("Ab1!xyz-"), &loaded("{}\n")).is_err());
    let floor = loaded("general_settings:\n  password_policy_min_length: 1\n");
    assert_eq!(validate(&secret("Ab1!xyz-"), &floor), Ok(()));
}

#[test]
fn errors_carry_the_validation_contract() {
    assert_eq!(
        (STATUS, ERROR_TYPE, PARAM),
        (400, "validation_error", "password")
    );
    assert_eq!(
        PasswordError::Breached.to_string(),
        "This password appears in known data breaches and cannot be used. Please choose a different password."
    );
}

#[derive(Debug, Clone)]
struct Seen {
    prefix: String,
    padding: Option<String>,
    agent: Option<String>,
}

#[derive(Clone)]
struct Mock {
    replies: Arc<BTreeMap<String, (u16, Vec<u8>)>>,
    delay: Duration,
    seen: Arc<Mutex<Vec<Seen>>>,
}

async fn range(
    State(mock): State<Mock>,
    Path(prefix): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Vec<u8>) {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    mock.seen.lock().unwrap().push(Seen {
        prefix: prefix.clone(),
        padding: header("add-padding"),
        agent: header("user-agent"),
    });
    sleep(mock.delay).await;
    match mock.replies.get(&prefix) {
        Some((status, body)) => (StatusCode::from_u16(*status).unwrap(), body.clone()),
        None => (StatusCode::OK, Vec::new()),
    }
}

struct Hibp {
    checker: BreachChecker,
    seen: Arc<Mutex<Vec<Seen>>>,
    _server: JoinSet<()>,
}

impl Hibp {
    fn prefixes(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|seen| seen.prefix.clone())
            .collect()
    }
}

fn client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

async fn hibp(replies: &[(&str, u16, &str)], delay: Duration, timeout: Duration) -> Hibp {
    let bytes: Vec<(&str, u16, &[u8])> = replies
        .iter()
        .map(|(prefix, status, body)| (*prefix, *status, body.as_bytes()))
        .collect();
    hibp_bytes(&bytes, delay, timeout).await
}

async fn hibp_bytes(replies: &[(&str, u16, &[u8])], delay: Duration, timeout: Duration) -> Hibp {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mock = Mock {
        replies: Arc::new(
            replies
                .iter()
                .map(|(prefix, status, body)| ((*prefix).to_owned(), (*status, (*body).to_vec())))
                .collect(),
        ),
        delay,
        seen: Arc::clone(&seen),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/range", listener.local_addr().unwrap());
    let application = Router::new()
        .route("/range/{prefix}", get(range))
        .with_state(mock);
    let mut server = JoinSet::new();
    server.spawn(async move {
        axum::serve(listener, application).await.unwrap();
    });
    Hibp {
        checker: BreachChecker {
            client: client(),
            base,
            timeout,
            enabled: true,
        },
        seen,
        _server: server,
    }
}

const PASSWORD_RANGE: &str = "0018A45C4D1DEF81644B54AB7F969B88D65:3\r\n1E4C9B93F3F0682250B6CF8331B7EE68FD8:3730471\r\n011053FD0102E94D6AE2F8B83D76FAF94F6:0";
const ODD_COUNT: &str = "1E4C9B93F3F0682250B6CF8331B7EE68FD8:1_0";

fn relaxed() -> Settings {
    Settings {
        min_length: 8,
        required: Vec::new(),
        check_breached_passwords: true,
    }
}

#[tokio::test]
async fn breached_password() {
    let server = hibp(
        &[("5BAA6", 200, PASSWORD_RANGE)],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .await;
    let breached = server
        .checker
        .validate_not_breached(&secret("password"))
        .await;
    assert_eq!(breached, Err(PasswordError::Breached));
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].prefix, "5BAA6");
    assert_eq!(seen[0].padding.as_deref(), Some("true"));
    assert_eq!(seen[0].agent.as_deref(), Some("litellm-proxy/1.105.1"));
    drop(seen);

    let clean = server
        .checker
        .is_breached(&secret("Tr0ub4dor&3-horse"))
        .await;
    assert!(!clean);
    assert_eq!(server.prefixes(), ["5BAA6", "0F58E"]);
    assert_eq!(HIBP_RANGE_API_BASE, "https://api.pwnedpasswords.com/range");
    assert_eq!(HIBP_TIMEOUT, Duration::from_secs(5));
    assert_eq!(validate(&secret("password"), &relaxed()), Ok(()));
}

#[rstest]
#[case::listed("ABC:2\n1E4C9B93F3F0682250B6CF8331B7EE68FD8:2", true)]
#[case::lowercase_body("1e4c9b93f3f0682250b6cf8331b7ee68fd8:5", true)]
#[case::padding_entry_with_zero("1E4C9B93F3F0682250B6CF8331B7EE68FD8:0", false)]
#[case::missing_count("1E4C9B93F3F0682250B6CF8331B7EE68FD8", false)]
#[case::blank_count("1E4C9B93F3F0682250B6CF8331B7EE68FD8:  ", false)]
#[case::padded_line("  1E4C9B93F3F0682250B6CF8331B7EE68FD8: 7  \r\n", true)]
#[case::space_before_colon_is_not_trimmed("1E4C9B93F3F0682250B6CF8331B7EE68FD8 :7", false)]
#[case::unicode_line_break("X:1\u{2028}1E4C9B93F3F0682250B6CF8331B7EE68FD8:1", true)]
#[case::other_suffix("1E4C9B93F3F0682250B6CF8331B7EE68FD9:9", false)]
#[case::longer_suffix("1E4C9B93F3F0682250B6CF8331B7EE68FD8X:9", false)]
#[case::empty_body("", false)]
#[tokio::test]
async fn range_body_matching(#[case] body: &str, #[case] breached: bool) {
    let server = hibp(
        &[("5BAA6", 200, body)],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(
        server.checker.lookup(&secret("password")).await.unwrap(),
        breached
    );
}

#[tokio::test]
async fn disabled_check_sends_nothing() {
    let mut server = hibp(
        &[("5BAA6", 200, PASSWORD_RANGE)],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .await;
    server.checker.enabled = false;
    assert!(!server.checker.is_breached(&secret("password")).await);
    assert_eq!(
        server
            .checker
            .validate_not_breached(&secret("password"))
            .await,
        Ok(())
    );
    assert!(server.prefixes().is_empty());
}

fn capture_warnings() -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (log, set_default(subscriber))
}

const SKIPPED: &str = "Breached-password check skipped, HIBP lookup failed";

#[tokio::test]
async fn hibp_timeout_warning_allow() {
    let (log, _guard) = capture_warnings();
    let slow = hibp(
        &[("5BAA6", 200, PASSWORD_RANGE)],
        Duration::from_secs(5),
        Duration::from_millis(150),
    )
    .await;
    let started = Instant::now();
    assert!(!slow.checker.is_breached(&secret("password")).await);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(matches!(
        slow.checker.lookup(&secret("password")).await,
        Err(LookupError::Transport(_))
    ));
    assert_eq!(
        slow.checker
            .validate_not_breached(&secret("password"))
            .await,
        Ok(())
    );

    let unreachable = BreachChecker {
        base: "http://127.0.0.1:1/range".to_owned(),
        ..slow.checker
    };
    assert!(!unreachable.is_breached(&secret("password")).await);

    let output = read_to_string(log.path()).unwrap();
    assert_eq!(output.matches(SKIPPED).count(), 3);
    assert!(output.contains("WARN"));
    assert!(!output.contains("password\""));
}

#[tokio::test]
async fn bad_range_answers_fail_open_with_a_warning() {
    let (log, _guard) = capture_warnings();
    let broken = hibp(
        &[("5BAA6", 503, PASSWORD_RANGE)],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .await;
    assert!(!broken.checker.is_breached(&secret("password")).await);
    assert!(matches!(
        broken.checker.lookup(&secret("password")).await,
        Err(LookupError::Status { status: 503 })
    ));

    let garbled = hibp(
        &[("5BAA6", 200, "1E4C9B93F3F0682250B6CF8331B7EE68FD8:many")],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .await;
    assert!(!garbled.checker.is_breached(&secret("password")).await);
    assert!(matches!(
        garbled.checker.lookup(&secret("password")).await,
        Err(LookupError::Count { .. })
    ));

    let output = read_to_string(log.path()).unwrap();
    assert_eq!(output.matches(SKIPPED).count(), 2);
}

#[tokio::test]
async fn invalid_utf8_body_fails_open_with_a_warning() {
    let (log, _guard) = capture_warnings();
    let garbled = hibp_bytes(
        &[("5BAA6", 200, b"1\xff")],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .await;
    assert!(!garbled.checker.is_breached(&secret("password")).await);
    assert!(matches!(
        garbled.checker.lookup(&secret("password")).await,
        Err(LookupError::Body(_))
    ));
    let output = read_to_string(log.path()).unwrap();
    assert_eq!(output.matches(SKIPPED).count(), 1);
}

#[tokio::test]
async fn non_2xx_and_odd_counts_fail_open_with_a_warning() {
    let (log, _guard) = capture_warnings();
    let denied = hibp(
        &[("5BAA6", 302, PASSWORD_RANGE)],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .await;
    let odd = hibp(
        &[("5BAA6", 200, ODD_COUNT)],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .await;
    assert!(!denied.checker.is_breached(&secret("password")).await);
    assert!(matches!(
        denied.checker.lookup(&secret("password")).await,
        Err(LookupError::Status { status: 302 })
    ));
    assert!(!odd.checker.is_breached(&secret("password")).await);
    assert!(matches!(
        odd.checker.lookup(&secret("password")).await,
        Err(LookupError::Count { .. })
    ));
    let output = read_to_string(log.path()).unwrap();
    assert_eq!(output.matches(SKIPPED).count(), 2);
}

#[tokio::test]
async fn bulk_deduplicated_password() {
    let strong = "Corr3ct-Horse-Battery!";
    let fine = "Tr0ub4dor&3-horse";
    let server = hibp(
        &[
            ("57439", 200, "EB6E970E3812370C5209B6CA1D382E198BB:12"),
            ("0F58E", 200, "947D95C7192CC0EED07F1EAA1875CBF160D:0"),
        ],
        Duration::from_millis(300),
        Duration::from_secs(5),
    )
    .await;
    let batch = [
        secret(strong),
        secret("short"),
        secret(fine),
        secret(strong),
        secret("short"),
        secret(fine),
    ];
    let started = Instant::now();
    let verdicts = validate_passwords_bulk(&batch, &defaults(), &server.checker).await;
    assert!(started.elapsed() < Duration::from_millis(580));

    assert_eq!(verdicts.len(), 3);
    assert_eq!(verdicts[0].error, Some(PasswordError::Breached));
    assert_eq!(
        verdicts[1].error,
        Some(PasswordError::Policy {
            requirements: "be at least 12 characters long, include an uppercase letter, include a number, include a special character".to_owned()
        })
    );
    assert_eq!(verdicts[2].error, None);
    let mut prefixes = server.prefixes();
    prefixes.sort();
    assert_eq!(prefixes, ["0F58E", "57439"]);
}

#[tokio::test]
async fn bulk_fails_open_per_password() {
    let server = hibp(
        &[
            ("57439", 500, ""),
            ("0F58E", 200, "947D95C7192CC0EED07F1EAA1875CBF160D:4"),
        ],
        Duration::ZERO,
        Duration::from_secs(5),
    )
    .await;
    let batch = [
        secret("Corr3ct-Horse-Battery!"),
        secret("Tr0ub4dor&3-horse"),
    ];
    let verdicts = validate_passwords_bulk(&batch, &defaults(), &server.checker).await;
    assert_eq!(verdicts[0].error, None);
    assert_eq!(verdicts[1].error, Some(PasswordError::Breached));
}

struct Users {
    rows: Mutex<Vec<(String, String)>>,
    fail_on: Option<&'static str>,
}

#[derive(Debug, thiserror::Error)]
#[error("write refused for {0}")]
struct Refused(String);

impl PasswordStore for Users {
    type Error = Refused;

    fn users_with_passwords(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredPassword>, Refused>> + Send {
        let rows = self.rows.lock().unwrap();
        ready(Ok(rows
            .iter()
            .map(|(user_id, password)| StoredPassword {
                user_id: user_id.clone(),
                password: secret(password),
            })
            .collect()))
    }

    fn set_password(
        &self,
        user_id: &str,
        stored_hash: &str,
    ) -> impl Future<Output = Result<(), Refused>> + Send {
        if self.fail_on == Some(user_id) {
            return ready(Err(Refused(user_id.to_owned())));
        }
        let mut rows = self.rows.lock().unwrap();
        for row in rows.iter_mut().filter(|row| row.0 == user_id) {
            row.1 = stored_hash.to_owned();
        }
        drop(rows);
        ready(Ok(()))
    }
}

fn users(rows: &[(&str, &str)], fail_on: Option<&'static str>) -> Users {
    Users {
        rows: Mutex::new(
            rows.iter()
                .map(|(user, password)| ((*user).to_owned(), (*password).to_owned()))
                .collect(),
        ),
        fail_on,
    }
}

#[tokio::test]
async fn migration_hashes_only_plaintext_rows() {
    let hasher = PasswordHasher::new(FixedSalts, 2);
    let store = users(
        &[
            ("a", "hunter2"),
            ("b", PYTHON_SHA256),
            ("c", PYTHON_SCRYPT),
            ("d", ""),
            ("e", "Another$Strong8passw"),
            ("f", "ABCDEF"),
        ],
        None,
    );
    let message = migrate_passwords_to_scrypt(&store, &hasher).await.unwrap();
    assert_eq!(message, "Migrated 3 plaintext passwords to scrypt");
    let rows = store.rows.lock().unwrap().clone();
    assert!(hasher.verify(&secret("hunter2"), &rows[0].1).await);
    assert_eq!(rows[1].1, PYTHON_SHA256);
    assert_eq!(rows[2].1, PYTHON_SCRYPT);
    assert_eq!(rows[3].1, "");
    assert!(
        hasher
            .verify(&secret("Another$Strong8passw"), &rows[4].1)
            .await
    );
    assert!(hasher.verify(&secret("ABCDEF"), &rows[5].1).await);
    let again = migrate_passwords_to_scrypt(&store, &hasher).await.unwrap();
    assert_eq!(again, "No plaintext passwords found");
}

#[tokio::test]
async fn migration_stops_at_the_first_failed_write() {
    let hasher = PasswordHasher::new(FixedSalts, 1);
    let store = users(&[("a", "one"), ("b", "two"), ("c", "three")], Some("b"));
    let error = migrate_passwords_to_scrypt(&store, &hasher)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "the user store failed");
    let rows = store.rows.lock().unwrap().clone();
    assert!(rows[0].1.starts_with("scrypt:"));
    assert_eq!(rows[1].1, "two");
    assert_eq!(rows[2].1, "three");
}
