#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::NamedTempFile;
use tracing::subscriber::with_default;

use super::{
    CA_CERT_ENV, CLIENT_CERT_ENV, CLIENT_KEY_ENV, ENDPOINT_ENV, EXPORT_INTERVAL_ENV, Metering, Raw,
    Settings,
};
use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::Section as _;

const CERT_PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIBcert\n-----END CERTIFICATE-----";
const KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIBsecretkey\n-----END PRIVATE KEY-----\n";

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

fn settings(pairs: &[(&str, &str)]) -> Settings {
    let environment: BTreeMap<String, String> = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Raw::default().convert(&environment).unwrap()
}

fn credential_files(directory: &Path) -> [String; 3] {
    [
        ("client.pem", CERT_PEM),
        ("client-key.pem", KEY_PEM),
        ("ca.pem", CERT_PEM),
    ]
    .map(|(name, pem)| {
        let path = directory.join(name);
        fs::write(&path, pem).unwrap();
        path.to_str().unwrap().to_owned()
    })
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn billing_metrics_config_reads_every_variable() {
    let configured = settings(&[
        (ENDPOINT_ENV, "https://collector"),
        (CLIENT_CERT_ENV, "/cert"),
        (CLIENT_KEY_ENV, "/key"),
        (CA_CERT_ENV, "/ca"),
        (EXPORT_INTERVAL_ENV, "1234"),
        ("TMPDIR", "/scratch"),
    ]);
    assert_eq!(configured.endpoint.as_deref(), Some("https://collector"));
    assert_eq!(configured.client_cert.as_deref(), Some("/cert"));
    assert_eq!(configured.ca_cert.as_deref(), Some("/ca"));
    assert_eq!(configured.export_interval_ms, 1234);
    assert_eq!(
        configured.temp_root_candidates[0],
        PathBuf::from("/scratch")
    );
    assert!(
        configured
            .client_key
            .is_some_and(|key| key.expose_secret() == "/key")
    );
}

#[test]
fn billing_metrics_config_treats_empty_values_as_unset() {
    let names = [
        ENDPOINT_ENV,
        CLIENT_CERT_ENV,
        CLIENT_KEY_ENV,
        CA_CERT_ENV,
        "TMPDIR",
    ];
    let configured = settings(&names.map(|name| (name, "")));
    assert_eq!((configured.endpoint, configured.client_cert), (None, None));
    assert!(configured.client_key.is_none() && configured.ca_cert.is_none());
    assert_eq!(
        configured.temp_root_candidates,
        Settings::default().temp_root_candidates
    );
    assert_eq!(Settings::default().export_interval_ms, 60_000);
}

#[rstest]
#[case::unset(None, 60_000, false)]
#[case::number(Some("1234"), 1234, false)]
#[case::padded(Some(" 250\n"), 250, false)]
#[case::plus(Some("+250"), 250, false)]
#[case::zero(Some("0"), 0, false)]
#[case::negative(Some("-5"), -5, false)]
#[case::sign_only(Some("+"), 60_000, true)]
#[case::text(Some("soon"), 60_000, true)]
#[case::fraction(Some("1.5"), 60_000, true)]
#[case::empty(Some(""), 60_000, true)]
#[case::underscored(Some("5_000"), 5000, false)]
#[case::negative_underscored(Some("-1_234"), -1234, false)]
#[case::unicode_digits(Some("\u{0665}\u{0660}\u{0660}\u{0660}"), 5000, false)]
#[case::leading_underscore(Some("_1000"), 60_000, true)]
#[case::trailing_underscore(Some("1000_"), 60_000, true)]
#[case::doubled_underscore(Some("1__000"), 60_000, true)]
fn billing_metrics_config_parses_the_export_interval(
    #[case] raw: Option<&str>,
    #[case] expected: i64,
    #[case] warns: bool,
) {
    let pairs: Vec<(&str, &str)> = raw
        .map(|value| (EXPORT_INTERVAL_ENV, value))
        .into_iter()
        .collect();
    let (configured, output) = logged(|| settings(&pairs));
    assert_eq!(configured.export_interval_ms, expected);
    let warning = format!(
        "Invalid LITELLM_BILLING_METRICS_EXPORT_INTERVAL_MS={:?}, falling back to 60000 ms",
        raw.unwrap_or_default()
    );
    assert_eq!(output.contains(&warning), warns, "{output}");
    assert_eq!(output.contains("WARN"), warns, "{output}");
}

#[rstest]
#[case::tmpdir(&[("TMP", "/c"), ("TEMP", "/b"), ("TMPDIR", "/a")], &["/a", "/b", "/c"])]
#[case::temp(&[("TMP", "/c"), ("TEMP", "/b")], &["/b", "/c"])]
#[case::tmp(&[("TMP", "/c")], &["/c"])]
#[case::fallback(&[], &[])]
fn billing_metrics_config_picks_the_temp_root(
    #[case] pairs: &[(&str, &str)],
    #[case] expected_prefix: &[&str],
) {
    let candidates = settings(pairs).temp_root_candidates;
    let prefix: Vec<PathBuf> = expected_prefix.iter().map(PathBuf::from).collect();
    assert_eq!(&candidates[..prefix.len()], prefix.as_slice());
    assert_eq!(candidates[prefix.len()], PathBuf::from("/tmp"));
    assert!(candidates.contains(&PathBuf::from("/var/tmp")));
}

#[test]
fn billing_metrics_config_uses_the_current_directory_after_platform_defaults() {
    let candidates = settings(&[]).temp_root_candidates;
    assert_eq!(candidates.last(), Some(&env::current_dir().unwrap()));
}

#[rstest]
#[case::nothing(&[], "(LITELLM_BILLING_METRICS_ENDPOINT, LITELLM_BILLING_METRICS_CLIENT_CERT, LITELLM_BILLING_METRICS_CLIENT_KEY)")]
#[case::endpoint(&[(CLIENT_CERT_ENV, "/c"), (CLIENT_KEY_ENV, "/k")], "(LITELLM_BILLING_METRICS_ENDPOINT)")]
#[case::cert(&[(ENDPOINT_ENV, "https://c"), (CLIENT_KEY_ENV, "/k")], "(LITELLM_BILLING_METRICS_CLIENT_CERT)")]
#[case::key(&[(ENDPOINT_ENV, "https://c"), (CLIENT_CERT_ENV, "/c")], "(LITELLM_BILLING_METRICS_CLIENT_KEY)")]
fn billing_metrics_config_names_missing_variables(
    #[case] pairs: &[(&str, &str)],
    #[case] missing: &str,
) {
    let configured = settings(pairs);
    let (metering, output) = logged(|| configured.resolve());
    assert_eq!(metering, None);
    assert!(
        output.contains("WARN") && output.contains(missing),
        "{output}"
    );
    assert!(!output.contains("/k") && !output.contains("https://c"));
}

#[test]
fn billing_metrics_config_keeps_file_paths() {
    let directory = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let [cert, key, authority] = credential_files(directory.path());
    let configured = settings(&[
        (ENDPOINT_ENV, "https://collector/"),
        (CLIENT_CERT_ENV, &cert),
        (CLIENT_KEY_ENV, &key),
        (CA_CERT_ENV, &authority),
        (EXPORT_INTERVAL_ENV, "500"),
        ("TMPDIR", scratch.path().to_str().unwrap()),
    ]);
    let (metering, output) = logged(|| configured.resolve());
    let expected = Metering {
        endpoint: "https://collector/v1/metrics".to_owned(),
        client_cert: PathBuf::from(cert),
        client_key: PathBuf::from(key),
        ca_cert: Some(PathBuf::from(authority)),
        export_interval_ms: 500,
    };
    assert_eq!(metering, Some(expected));
    assert_eq!(output, "");
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
}

#[test]
fn billing_metrics_config_writes_inline_pem_privately() {
    let directory = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let [cert, _key, _authority] = credential_files(directory.path());
    let indented_authority = format!("\n  {CERT_PEM}");
    let configured = settings(&[
        (ENDPOINT_ENV, "https://collector"),
        (CLIENT_CERT_ENV, &cert),
        (CLIENT_KEY_ENV, KEY_PEM),
        (CA_CERT_ENV, &indented_authority),
        ("TMPDIR", scratch.path().to_str().unwrap()),
    ]);
    let metering = configured.resolve().unwrap();
    let written = metering.client_key.parent().unwrap().to_path_buf();
    assert_eq!(written.parent().unwrap(), scratch.path());
    let name = written.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("betterlite-billing-mtls-"), "{name}");
    assert_eq!(metering.client_cert, PathBuf::from(cert));
    assert_eq!(metering.client_key, written.join("client.key"));
    assert_eq!(metering.ca_cert, Some(written.join("ca.crt")));
    assert_eq!((mode(&written), mode(&metering.client_key)), (0o700, 0o600));
    assert_eq!(fs::read_to_string(&metering.client_key).unwrap(), KEY_PEM);
    let authority = fs::read_to_string(written.join("ca.crt")).unwrap();
    assert_eq!(authority, format!("{indented_authority}\n"));
    assert_eq!(fs::read_dir(&written).unwrap().count(), 2);
}

#[test]
fn billing_metrics_config_writes_inline_credentials_without_a_custom_ca() {
    let scratch = tempfile::tempdir().unwrap();
    let configured = settings(&[
        (ENDPOINT_ENV, "https://collector"),
        (CLIENT_CERT_ENV, CERT_PEM),
        (CLIENT_KEY_ENV, KEY_PEM),
        ("TMPDIR", scratch.path().to_str().unwrap()),
    ]);
    let metering = configured.resolve().unwrap();
    assert!(metering.ca_cert.is_none());
    assert_eq!(
        fs::read_to_string(metering.client_cert).unwrap(),
        format!("{CERT_PEM}\n")
    );
    assert_eq!(fs::read_to_string(metering.client_key).unwrap(), KEY_PEM);
}

#[test]
fn billing_metrics_config_names_every_unreadable_path() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("absent.pem");
    let configured = settings(&[
        (ENDPOINT_ENV, "https://collector"),
        (CLIENT_CERT_ENV, missing.to_str().unwrap()),
        (CLIENT_KEY_ENV, directory.path().to_str().unwrap()),
        (CA_CERT_ENV, "/nowhere/ca.pem"),
    ]);
    let (metering, output) = logged(|| configured.resolve());
    assert_eq!(metering, None);
    let named = "LITELLM_BILLING_METRICS_CLIENT_CERT, LITELLM_BILLING_METRICS_CLIENT_KEY, LITELLM_BILLING_METRICS_CA_CERT did not resolve to a readable certificate file";
    assert!(output.contains(named), "{output}");
    assert!(!output.contains("absent.pem") && !output.contains("nowhere"));
}

#[test]
fn billing_metrics_config_reports_write_failures() {
    // every candidate is exhausted, which TMPDIR alone cannot force once /tmp exists
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("not-a-directory");
    fs::write(&blocker, "plain file").unwrap();
    let configured = Settings {
        endpoint: Some("https://collector".to_owned()),
        client_cert: Some(CERT_PEM.to_owned()),
        client_key: Some(SecretString::from(KEY_PEM)),
        ca_cert: None,
        export_interval_ms: 60_000,
        temp_root_candidates: vec![blocker.clone(), blocker],
    };
    let (metering, output) = logged(|| configured.resolve());
    assert_eq!(metering, None);
    assert!(
        output.contains("WARN") && output.contains("Not a directory"),
        "{output}"
    );
    let failure = "Billing metrics disabled: could not write inline certificate content to disk";
    assert!(output.contains(failure), "{output}");
    assert!(
        !output.contains("MIIBsecretkey") && !format!("{configured:?}").contains("MIIBsecretkey")
    );
}

#[test]
fn billing_metrics_config_reports_missing_temp_directory_candidates() {
    let error = super::credentials::resolve_paths(&[], CERT_PEM, KEY_PEM, None).unwrap_err();
    assert!(matches!(
        error,
        super::credentials::CredentialError::NoTemporaryDirectory
    ));
}

#[test]
fn billing_metrics_config_falls_back_when_tmpdir_is_unusable() {
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("not-a-directory");
    fs::write(&blocker, "plain file").unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let configured = settings(&[
        (ENDPOINT_ENV, "https://collector"),
        (CLIENT_CERT_ENV, CERT_PEM),
        (CLIENT_KEY_ENV, KEY_PEM),
        ("TMPDIR", blocker.to_str().unwrap()),
        ("TEMP", scratch.path().to_str().unwrap()),
    ]);
    let (metering, output) = logged(|| configured.resolve());
    assert!(!output.contains("WARN"), "{output}");
    let written = metering.unwrap().client_key.parent().unwrap().to_path_buf();
    assert_eq!(written.parent().unwrap(), scratch.path());
}

#[rstest]
#[case::bare("https://collector", "https://collector/v1/metrics")]
#[case::slashes("https://collector///", "https://collector/v1/metrics")]
#[case::suffixed("https://collector/v1/metrics", "https://collector/v1/metrics")]
#[case::suffixed_slash("https://collector/v1/metrics/", "https://collector/v1/metrics")]
#[case::prefixed("https://collector/otlp", "https://collector/otlp/v1/metrics")]
fn billing_metrics_config_suffixes_the_endpoint(#[case] endpoint: &str, #[case] expected: &str) {
    let directory = tempfile::tempdir().unwrap();
    let [cert, key, _authority] = credential_files(directory.path());
    let configured = settings(&[
        (ENDPOINT_ENV, endpoint),
        (CLIENT_CERT_ENV, &cert),
        (CLIENT_KEY_ENV, &key),
    ]);
    assert_eq!(configured.resolve().unwrap().endpoint, expected);
}

#[test]
fn billing_metrics_config_has_no_yaml_keys() {
    let loader = ConfigLoader {
        lookup: BTreeMap::new(),
    };
    let error = loader
        .load("billing_metrics:\n  unexpected: true\n")
        .unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { .. }));
}
