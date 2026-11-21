#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::io::Error as IoError;
use std::sync::{Arc, Mutex};

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};

use super::resolve_json::resolved;
use super::{DEFAULT_USER_PARAMS_PATH, ROLE_PERMISSIONS_PATH, RawValue, RbacRole, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .auth_user_lookup)
}

fn scalar(value: &JsonData) -> Option<f64> {
    match value {
        JsonData::Number(number) => number.as_f64(),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::String(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}

fn scalar_error_path(source: &str) -> String {
    match load(source, &[]) {
        Err(ConfigError::InvalidScalar { path, .. }) => path,
        other => panic!("the setting should fail startup as an invalid scalar: {other:?}"),
    }
}

#[test]
fn loads_role_permissions() {
    let settings = load(
        "general_settings:
  role_permissions:
    - role: proxy_admin
      models: [gpt-4]
      routes: [\"/user/new\"]
    - role: team
",
        &[],
    )
    .unwrap();
    let configured = settings.role_permissions.unwrap();
    assert_eq!(configured.len(), 2);
    assert_eq!(configured[0].role, RbacRole::ProxyAdmin);
    assert_eq!(configured[0].models, Some(vec!["gpt-4".to_owned()]));
    assert_eq!(configured[0].routes, Some(vec!["/user/new".to_owned()]));
    assert_eq!(configured[1].role, RbacRole::Team);
    assert_eq!(configured[1].models, None);
    assert_eq!(configured[1].routes, None);
}

#[test]
fn loads_scalar_user_defaults() {
    let settings = load(
        "litellm_settings:
  default_internal_user_params:
    user_role: internal_user
    max_budget: 12
    metadata:
      team: core
",
        &[],
    )
    .unwrap();
    let defaults = settings.default_internal_user_params.unwrap();
    assert_eq!(
        defaults.values.get("user_role"),
        Some(&JsonData::String("internal_user".to_owned()))
    );
    assert_eq!(
        scalar(defaults.values.get("max_budget").unwrap()),
        Some(12.0_f64)
    );
    match defaults.values.get("metadata") {
        Some(JsonData::Object(fields)) => {
            assert_eq!(
                fields.get("team"),
                Some(&JsonData::String("core".to_owned()))
            );
        }
        other => panic!("metadata should load as a mapping: {other:?}"),
    }
}

#[test]
fn omitted_user_settings_defaults() {
    let settings = load("", &[]).unwrap();
    assert!(settings.role_permissions.is_none());
    assert!(settings.default_internal_user_params.is_none());
}

#[test]
fn settings_env_references() {
    let settings = load(
        "general_settings:
  role_permissions:
    - role: os.environ/RBAC_ROLE
litellm_settings:
  default_internal_user_params:
    user_role: os.environ/DEFAULT_ROLE
    max_budget: os.environ/DEFAULT_BUDGET
",
        &[
            ("RBAC_ROLE", "team"),
            ("DEFAULT_ROLE", "internal_user"),
            ("DEFAULT_BUDGET", "42.5"),
        ],
    )
    .unwrap();
    assert_eq!(settings.role_permissions.unwrap()[0].role, RbacRole::Team);
    let defaults = settings.default_internal_user_params.unwrap();
    assert_eq!(
        defaults.values.get("user_role"),
        Some(&JsonData::String("internal_user".to_owned()))
    );
    assert_eq!(
        scalar(defaults.values.get("max_budget").unwrap()),
        Some(42.5_f64)
    );
    match load(
        "litellm_settings:
  default_internal_user_params:
    max_budget: os.environ/MISSING_BUDGET
",
        &[],
    ) {
        Err(ConfigError::MissingEnvironment { variable, .. }) => {
            assert_eq!(variable, "MISSING_BUDGET");
        }
        other => panic!("a missing variable should fail startup: {other:?}"),
    }
}

#[rstest]
#[case::unknown_role("admin")]
#[case::capitalized("Proxy_Admin")]
#[case::empty("")]
fn malformed_role_stops_startup(#[case] role: &str) {
    let source = format!(
        "general_settings:
  role_permissions:
    - role: proxy_admin
    - role: \"{role}\"
"
    );
    assert_eq!(scalar_error_path(&source), ROLE_PERMISSIONS_PATH);
}

#[rstest]
#[case::empty_mapping("{}")]
#[case::null("null")]
#[case::disabled("false")]
#[case::zero("0")]
#[case::empty_text("\"\"")]
#[case::empty_list("[]")]
fn falsey_defaults_empty(#[case] value: &str) {
    let source = format!("litellm_settings:\n  default_internal_user_params: {value}\n");
    let settings = load(&source, &[]).unwrap();
    assert!(settings.default_internal_user_params.is_none());
}

#[rstest]
#[case::integer("12", 12.0_f64)]
#[case::fractional("12.5", 12.5_f64)]
#[case::numeric_text("\"7.25\"", 7.25_f64)]
#[case::source_true("true", 1.0_f64)]
#[case::source_false("false", 0.0_f64)]
#[case::zero("0", 0.0_f64)]
fn defaults_numeric_budget_conversion(#[case] written: &str, #[case] expected: f64) {
    let source =
        format!("litellm_settings:\n  default_internal_user_params:\n    max_budget: {written}\n");
    let settings = load(&source, &[]).unwrap();
    let defaults = settings.default_internal_user_params.unwrap();
    let converted = scalar(defaults.values.get("max_budget").unwrap()).unwrap();
    assert_eq!(converted.to_bits(), expected.to_bits());
}

#[rstest]
#[case::text("\"abc\"")]
#[case::empty_text("\"\"")]
#[case::list("[]")]
#[case::mapping("{}")]
fn invalid_budget_stops_startup(#[case] written: &str) {
    let source =
        format!("litellm_settings:\n  default_internal_user_params:\n    max_budget: {written}\n");
    assert_eq!(
        scalar_error_path(&source),
        format!("{DEFAULT_USER_PARAMS_PATH}.max_budget")
    );
}

#[rstest]
#[case::enabled("true")]
#[case::text("\"enabled\"")]
#[case::number("5")]
#[case::list("[1, 2]")]
fn nonmapping_defaults_named_error(#[case] written: &str) {
    let source = format!("litellm_settings:\n  default_internal_user_params: {written}\n");
    assert_eq!(scalar_error_path(&source), DEFAULT_USER_PARAMS_PATH);
}

#[rstest]
#[case::nan_text("\"nan\"")]
#[case::infinity_text("\"inf\"")]
#[case::negative_infinity_text("\"-inf\"")]
#[case::python_spelling("\"Infinity\"")]
#[case::overflowing_exponent("\"1e999\"")]
fn nonfinite_budget_named_error(#[case] written: &str) {
    let source =
        format!("litellm_settings:\n  default_internal_user_params:\n    max_budget: {written}\n");
    assert_eq!(
        scalar_error_path(&source),
        format!("{DEFAULT_USER_PARAMS_PATH}.max_budget")
    );
}

// a YAML non-finite float never reaches the section: the YAML layer refuses it first
#[test]
fn nonfinite_yaml_budget_fails_at_the_yaml_layer() {
    let source = "litellm_settings:\n  default_internal_user_params:\n    max_budget: .inf\n";
    match load(source, &[]) {
        Err(error) => assert_eq!(error.category(), "yaml"),
        Ok(settings) => panic!("a non-finite YAML budget cannot load: {settings:?}"),
    }
}

#[test]
fn defaults_debug_redacted() {
    let settings = load(
        "litellm_settings:
  default_internal_user_params:
    api_key: sk-live-secret
    database_url: postgres://user:pass@host/db
    user_role: internal_user
    nested:
      authorization: Bearer secret-token
",
        &[],
    )
    .unwrap();
    let rendered = format!("{:?}", settings.default_internal_user_params.unwrap());
    assert!(rendered.contains("REDACTED"), "{rendered}");
    assert!(rendered.contains("authorization"), "{rendered}");
    assert!(!rendered.contains("sk-live-secret"), "{rendered}");
    assert!(
        !rendered.contains("postgres://user:pass@host/db"),
        "{rendered}"
    );
    assert!(!rendered.contains("Bearer secret-token"), "{rendered}");
    assert!(rendered.contains("internal_user"), "{rendered}");
}

#[test]
fn budget_reset_absent_from_scalar_defaults() {
    let settings = load(
        "litellm_settings:
  default_internal_user_params:
    user_alias: default-user
",
        &[],
    )
    .unwrap();
    let defaults = settings.default_internal_user_params.unwrap();
    assert!(!defaults.values.contains_key("budget_reset_at"));
}

// the YAML layer caps document depth first, so the resolver's own guard is tested directly
fn nested_value(depth: usize) -> RawValue {
    let mut value = RawValue::Integer(1);
    for _ in 0..depth {
        let mut fields = BTreeMap::new();
        fields.insert("k".to_owned(), value);
        value = RawValue::Object(fields);
    }
    value
}

#[test]
fn deeply_nested_defaults_up_to_the_cap_load() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let tree = resolved(nested_value(64), DEFAULT_USER_PARAMS_PATH, &lookup)
        .expect("a value at the depth cap resolves");
    assert!(matches!(tree, JsonData::Object(_)));
}

#[test]
fn defaults_nested_past_the_cap_fail() {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    match resolved(nested_value(65), DEFAULT_USER_PARAMS_PATH, &lookup) {
        Err(ConfigError::InvalidScalar { path, .. }) => {
            assert_eq!(path, DEFAULT_USER_PARAMS_PATH);
        }
        other => panic!("a mapping past the depth cap fails startup: {other:?}"),
    }
}

#[test]
fn environment_reference_in_a_plain_list_stays_text() {
    let settings = load(
        "litellm_settings:
  default_internal_user_params:
    models:
      - os.environ/PLAIN_MODEL
",
        &[("PLAIN_MODEL", "gpt-4")],
    )
    .unwrap();
    let defaults = settings.default_internal_user_params.unwrap();
    match defaults.values.get("models") {
        Some(JsonData::Array(items)) => {
            assert_eq!(items.len(), 1);
            assert_eq!(
                items[0],
                JsonData::String("os.environ/PLAIN_MODEL".to_owned())
            );
        }
        other => panic!("models should load as a list: {other:?}"),
    }
}

struct CapturedBuffer(Arc<Mutex<Vec<u8>>>);

impl io::Write for CapturedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut captured) = self.0.lock() else {
            return Err(IoError::other("the captured log buffer is poisoned"));
        };
        captured.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct LogCapture {
    buffer: Arc<Mutex<Vec<u8>>>,
    _guard: DefaultGuard,
}

fn capture_debug_log() -> LogCapture {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let writer = Arc::clone(&buffer);
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || CapturedBuffer(Arc::clone(&writer)))
        .finish();
    let guard = set_default(subscriber);
    LogCapture {
        buffer,
        _guard: guard,
    }
}

#[test]
fn user_defaults_diagnostic_is_redacted() {
    let capture = capture_debug_log();
    let settings = load(
        "litellm_settings:
  default_internal_user_params:
    api_key: sk-live-secret
    user_role: internal_user
",
        &[],
    )
    .unwrap();
    assert!(settings.default_internal_user_params.is_some());
    let logged =
        String::from_utf8(capture.buffer.lock().unwrap().clone()).expect("the log is UTF-8");
    assert!(
        logged.contains("litellm.default_internal_user_params is set"),
        "{logged}"
    );
    assert!(logged.contains("REDACTED"), "{logged}");
    assert!(!logged.contains("sk-live-secret"), "{logged}");
}
