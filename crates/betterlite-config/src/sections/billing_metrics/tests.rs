#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rstest::rstest;
use secrecy::ExposeSecret;
use tracing::subscriber::with_default;

use super::{DEFAULT_EXPORT_INTERVAL_MS, Metering, Raw, Settings};
use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::Section as _;

const CERT_PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIBcert\n-----END CERTIFICATE-----";
const KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIIBsecretkey\n-----END PRIVATE KEY-----\n";

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let result = with_default(subscriber, action);
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    (result, output)
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings(pairs: &[(&str, &str)]) -> Settings {
    Raw::default().convert(&environment(pairs)).unwrap()
}

fn credential_files(directory: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let cert = directory.join("client.pem");
    let key = directory.join("client-key.pem");
    let authority = directory.join("ca.pem");
    fs::write(&cert, CERT_PEM).unwrap();
    fs::write(&key, KEY_PEM).unwrap();
    fs::write(&authority, CERT_PEM).unwrap();
    (cert, key, authority)
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn billing_metrics_config_reads_every_variable() {
    let configured = settings(&[
        ("LITELLM_BILLING_METRICS_ENDPOINT", "https://collector"),
        ("LITELLM_BILLING_METRICS_CLIENT_CERT", "/cert"),
        ("LITELLM_BILLING_METRICS_CLIENT_KEY", "/key"),
        ("LITELLM_BILLING_METRICS_CA_CERT", "/ca"),
        ("LITELLM_BILLING_METRICS_EXPORT_INTERVAL_MS", "1234"),
        ("TMPDIR", "/scratch"),
    ]);
    assert_eq!(configured.endpoint.as_deref(), Some("https://collector"));
    assert_eq!(configured.client_cert.as_deref(), Some("/cert"));
    assert_eq!(
        configured.client_key.as_ref().map(ExposeSecret::expose_secret),
        Some("/key")
    );
    assert_eq!(configured.ca_cert.as_deref(), Some("/ca"));
    assert_eq!(configured.export_interval_ms, 1234);
    assert_eq!(configured.temp_root, PathBuf::from("/scratch"));
}

#[test]
fn billing_metrics_config_treats_empty_values_as_unset() {
    let configured = settings(&[
        ("LITELLM_BILLING_METRICS_ENDPOINT", ""),
        ("LITELLM_BILLING_METRICS_CLIENT_CERT", ""),
        ("LITELLM_BILLING_METRICS_CLIENT_KEY", ""),
        ("LITELLM_BILLING_METRICS_CA_CERT", ""),
        ("TMPDIR", ""),
    ]);
    assert_eq!(configured.endpoint, None);
    assert_eq!(configured.client_cert, None);
    assert!(configured.client_key.is_none());
    assert_eq!(configured.ca_cert, None);
    assert_eq!(configured.temp_root, PathBuf::from("/tmp"));
}

#[test]
fn billing_metrics_config_defaults_match_an_empty_environment() {
    let defaults = Settings::default();
    let configured = settings(&[]);
    assert_eq!(defaults.export_interval_ms, 60_000);
    assert_eq!(configured.export_interval_ms, defaults.export_interval_ms);
    assert_eq!(configured.temp_root, defaults.temp_root);
    assert_eq!(configured.endpoint, defaults.endpoint);
}

#[rstest]
#[case::unset(None, 60_000, false)]
#[case::number(Some("1234"), 1234, false)]
#[case::padded(Some(" 250\n"), 250, false)]
#[case::zero(Some("0"), 0, false)]
#[case::negative(Some("-5"), -5, false)]
#[case::text(Some("soon"), DEFAULT_EXPORT_INTERVAL_MS, true)]
#[case::fraction(Some("1.5"), DEFAULT_EXPORT_INTERVAL_MS, true)]
#[case::empty(Some(""), DEFAULT_EXPORT_INTERVAL_MS, true)]
fn billing_metrics_config_parses_the_export_interval(
    #[case] raw: Option<&str>,
    #[case] expected: i64,
    #[case] warns: bool,
) {
    let pairs: Vec<(&str, &str)> = raw
        .map(|value| ("LITELLM_BILLING_METRICS_EXPORT_INTERVAL_MS", value))
        .into_iter()
        .collect();
    let (configured, output) = logged(|| settings(&pairs));
    assert_eq!(configured.export_interval_ms, expected);
    assert_eq!(output.contains("WARN"), warns);
    if let (true, Some(value)) = (warns, raw) {
        assert!(output.contains(&format!(
            "Invalid LITELLM_BILLING_METRICS_EXPORT_INTERVAL_MS={value:?}, falling back to 60000 ms"
        )));
    }
}

#[rstest]
#[case::tmpdir(&[("TMP", "/c"), ("TEMP", "/b"), ("TMPDIR", "/a")], "/a")]
#[case::temp(&[("TMP", "/c"), ("TEMP", "/b")], "/b")]
#[case::tmp(&[("TMP", "/c")], "/c")]
#[case::fallback(&[], "/tmp")]
fn billing_metrics_config_picks_the_temp_root(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    assert_eq!(settings(pairs).temp_root, PathBuf::from(expected));
}

#[rstest]
#[case::nothing(&[], "LITELLM_BILLING_METRICS_ENDPOINT, LITELLM_BILLING_METRICS_CLIENT_CERT, LITELLM_BILLING_METRICS_CLIENT_KEY")]
#[case::endpoint(&[("LITELLM_BILLING_METRICS_CLIENT_CERT", "/c"), ("LITELLM_BILLING_METRICS_CLIENT_KEY", "/k")], "(LITELLM_BILLING_METRICS_ENDPOINT)")]
#[case::cert(&[("LITELLM_BILLING_METRICS_ENDPOINT", "https://c"), ("LITELLM_BILLING_METRICS_CLIENT_KEY", "/k")], "(LITELLM_BILLING_METRICS_CLIENT_CERT)")]
#[case::key(&[("LITELLM_BILLING_METRICS_ENDPOINT", "https://c"), ("LITELLM_BILLING_METRICS_CLIENT_CERT", "/c")], "(LITELLM_BILLING_METRICS_CLIENT_KEY)")]
fn billing_metrics_config_names_missing_variables(
    #[case] pairs: &[(&str, &str)],
    #[case] missing: &str,
) {
    let configured = settings(pairs);
    let (metering, output) = logged(|| configured.resolve());
    assert_eq!(metering, None);
    assert!(output.contains("WARN"));
    assert!(output.contains(missing), "{output}");
    assert!(!output.contains("/k") && !output.contains("https://c"));
}

#[test]
fn billing_metrics_config_keeps_file_paths() {
    let directory = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (cert, key, authority) = credential_files(directory.path());
    let configured = settings(&[
        ("LITELLM_BILLING_METRICS_ENDPOINT", "https://collector/"),
        ("LITELLM_BILLING_METRICS_CLIENT_CERT", cert.to_str().unwrap()),
        ("LITELLM_BILLING_METRICS_CLIENT_KEY", key.to_str().unwrap()),
        ("LITELLM_BILLING_METRICS_CA_CERT", authority.to_str().unwrap()),
        ("LITELLM_BILLING_METRICS_EXPORT_INTERVAL_MS", "500"),
        ("TMPDIR", scratch.path().to_str().unwrap()),
    ]);
    let (metering, output) = logged(|| configured.resolve());
    assert_eq!(
        metering,
        Some(Metering {
            endpoint: "https://collector/v1/metrics".to_owned(),
            client_cert: cert,
            client_key: key,
            ca_cert: Some(authority),
            export_interval_ms: 500,
        })
    );
    assert_eq!(output, "");
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
}

#[test]
fn billing_metrics_config_writes_inline_pem_privately() {
    let scratch = tempfile::tempdir().unwrap();
    let indented_cert = format!("\n  {CERT_PEM}");
    let configured = settings(&[
        ("LITELLM_BILLING_METRICS_ENDPOINT", "https://collector"),
        ("LITELLM_BILLING_METRICS_CLIENT_CERT", &indented_cert),
        ("LITELLM_BILLING_METRICS_CLIENT_KEY", KEY_PEM),
        ("LITELLM_BILLING_METRICS_CA_CERT", CERT_PEM),
        ("TMPDIR", scratch.path().to_str().unwrap()),
    ]);
    let metering = configured.resolve().unwrap();
    let directory = metering.client_cert.parent().unwrap().to_path_buf();
    assert_eq!(directory.parent().unwrap(), scratch.path());
    let name = directory.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("betterlite-billing-mtls-"), "{name}");
    assert_eq!(mode(&directory), 0o700);
    assert_eq!(metering.client_cert, directory.join("client.crt"));
    assert_eq!(metering.client_key, directory.join("client.key"));
    assert_eq!(metering.ca_cert, Some(directory.join("ca.crt")));
    assert_eq!(mode(&metering.client_cert), 0o600);
    assert_eq!(mode(&metering.client_key), 0o600);
    assert_eq!(
        fs::read_to_string(&metering.client_cert).unwrap(),
        format!("{indented_cert}\n")
    );
    assert_eq!(fs::read_to_string(&metering.client_key).unwrap(), KEY_PEM);
    assert_eq!(
        fs::read_to_string(directory.join("ca.crt")).unwrap(),
        format!("{CERT_PEM}\n")
    );
    assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 1);
}

#[test]
fn billing_metrics_config_writes_only_the_inline_values() {
    let directory = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let (cert, _key, _authority) = credential_files(directory.path());
    let configured = settings(&[
        ("LITELLM_BILLING_METRICS_ENDPOINT", "https://collector"),
        ("LITELLM_BILLING_METRICS_CLIENT_CERT", cert.to_str().unwrap()),
        ("LITELLM_BILLING_METRICS_CLIENT_KEY", KEY_PEM),
        ("TMPDIR", scratch.path().to_str().unwrap()),
    ]);
    let metering = configured.resolve().unwrap();
    assert_eq!(metering.client_cert, cert);
    assert!(metering.client_key.starts_with(scratch.path()));
    assert_eq!(metering.ca_cert, None);
    let written = metering.client_key.parent().unwrap();
    assert_eq!(fs::read_dir(written).unwrap().count(), 1);
}

#[test]
fn billing_metrics_config_names_unreadable_paths_without_values() {
    let directory = tempfile::tempdir().unwrap();
    let (_cert, key, _authority) = credential_files(directory.path());
    let missing = directory.path().join("absent.pem");
    let configured = settings(&[
        ("LITELLM_BILLING_METRICS_ENDPOINT", "https://collector"),
        ("LITELLM_BILLING_METRICS_CLIENT_CERT", missing.to_str().unwrap()),
        ("LITELLM_BILLING_METRICS_CLIENT_KEY", key.to_str().unwrap()),
        ("LITELLM_BILLING_METRICS_CA_CERT", directory.path().to_str().unwrap()),
    ]);
    let (metering, output) = logged(|| configured.resolve());
    assert_eq!(metering, None);
    assert!(output.contains(
        "LITELLM_BILLING_METRICS_CLIENT_CERT, LITELLM_BILLING_METRICS_CA_CERT did not resolve to a readable certificate file"
    ), "{output}");
    assert!(!output.contains("LITELLM_BILLING_METRICS_CLIENT_KEY"));
    assert!(!output.contains("absent.pem"));
}

#[rstest]
#[case::cert("LITELLM_BILLING_METRICS_CLIENT_CERT")]
#[case::key("LITELLM_BILLING_METRICS_CLIENT_KEY")]
#[case::authority("LITELLM_BILLING_METRICS_CA_CERT")]
fn billing_metrics_config_flags_each_unreadable_variable(#[case] variable: &str) {
    let directory = tempfile::tempdir().unwrap();
    let (cert, key, authority) = credential_files(directory.path());
    let mut pairs = environment(&[
        ("LITELLM_BILLING_METRICS_ENDPOINT", "https://collector"),
        ("LITELLM_BILLING_METRICS_CLIENT_CERT", cert.to_str().unwrap()),
        ("LITELLM_BILLING_METRICS_CLIENT_KEY", key.to_str().unwrap()),
        ("LITELLM_BILLING_METRICS_CA_CERT", authority.to_str().unwrap()),
    ]);
    pairs.insert(variable.to_owned(), "/nowhere/credential.pem".to_owned());
    let configured = Raw::default().convert(&pairs).unwrap();
    let (metering, output) = logged(|| configured.resolve());
    assert_eq!(metering, None);
    assert!(
        output.contains(&format!(
            "{variable} did not resolve to a readable certificate file"
        )),
        "{output}"
    );
}

#[test]
fn billing_metrics_config_reports_write_failures() {
    let directory = tempfile::tempdir().unwrap();
    let blocker = directory.path().join("not-a-directory");
    fs::write(&blocker, "plain file").unwrap();
    let configured = settings(&[
        ("LITELLM_BILLING_METRICS_ENDPOINT", "https://collector"),
        ("LITELLM_BILLING_METRICS_CLIENT_CERT", CERT_PEM),
        ("LITELLM_BILLING_METRICS_CLIENT_KEY", KEY_PEM),
        ("TMPDIR", blocker.to_str().unwrap()),
    ]);
    let (metering, output) = logged(|| configured.resolve());
    assert_eq!(metering, None);
    assert!(output.contains("WARN"));
    assert!(output.contains("could not create a private directory"), "{output}");
    assert!(output.contains("Not a directory"), "{output}");
    assert!(!output.contains("MIIBsecretkey"));
}

#[rstest]
#[case::bare("https://collector", "https://collector/v1/metrics")]
#[case::slashes("https://collector///", "https://collector/v1/metrics")]
#[case::suffixed("https://collector/v1/metrics", "https://collector/v1/metrics")]
#[case::suffixed_slash("https://collector/v1/metrics/", "https://collector/v1/metrics")]
#[case::prefixed("https://collector/otlp", "https://collector/otlp/v1/metrics")]
fn billing_metrics_config_suffixes_the_endpoint(#[case] endpoint: &str, #[case] expected: &str) {
    let directory = tempfile::tempdir().unwrap();
    let (cert, key, _authority) = credential_files(directory.path());
    let configured = settings(&[
        ("LITELLM_BILLING_METRICS_ENDPOINT", endpoint),
        ("LITELLM_BILLING_METRICS_CLIENT_CERT", cert.to_str().unwrap()),
        ("LITELLM_BILLING_METRICS_CLIENT_KEY", key.to_str().unwrap()),
    ]);
    assert_eq!(configured.resolve().unwrap().endpoint, expected);
}

#[test]
fn billing_metrics_config_hides_the_key_from_debug() {
    let configured = settings(&[("LITELLM_BILLING_METRICS_CLIENT_KEY", KEY_PEM)]);
    assert!(!format!("{configured:?}").contains("MIIBsecretkey"));
}

#[test]
fn billing_metrics_config_has_no_yaml_keys() {
    let error = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("billing_metrics:\n  unexpected: true\n")
    .unwrap_err();
    assert!(matches!(error, ConfigError::Unsupported { .. }));
}
