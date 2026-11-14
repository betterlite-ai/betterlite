#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, parse_view};
use crate::sections::Section as _;
use crate::tls::VerifySetting;

const CATO_CONFIG: &str = "guardrails:\n  - guardrail_name: cato\n    litellm_params:\n      guardrail: cato_networks\n      api_key: literal-key\n      api_base: https://cato.test\n      inspect_embeddings: true\n      ssl_verify: false\n";

fn resolve(
    source: &str,
    lookup: &BTreeMap<String, String>,
) -> Result<Vec<super::Configured>, ConfigError> {
    let (raw, _ignored) = parse_view::<super::Raw>(source)?;
    raw.convert(lookup)
}

#[test]
fn cato_projections_config_reads_every_projected_key() {
    let settings = resolve(CATO_CONFIG, &BTreeMap::new()).unwrap();
    let configured = settings.first().unwrap();
    assert_eq!(configured.guardrail_name, "cato");
    assert_eq!(
        configured.api_key.as_ref().unwrap().expose_secret(),
        "literal-key"
    );
    assert_eq!(configured.api_base.as_deref(), Some("https://cato.test"));
    assert!(configured.inspect_embeddings);
    assert_eq!(configured.ssl_verify, Some(VerifySetting::Bool(false)));
}

#[test]
fn cato_projections_config_leaves_absent_keys_unset() {
    let source = "guardrails:\n  - guardrail_name: cato\n    litellm_params:\n      guardrail: cato_networks\n";
    let settings = resolve(source, &BTreeMap::new()).unwrap();
    let configured = settings.first().unwrap();
    assert!(configured.api_key.is_none());
    assert!(configured.api_base.is_none());
    assert!(!configured.inspect_embeddings);
    assert!(configured.ssl_verify.is_none());
}

#[test]
fn cato_projections_config_skips_other_guardrail_kinds() {
    let source = CATO_CONFIG.replace("guardrail: cato_networks", "guardrail: aim");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    assert!(settings.is_empty());
}

#[test]
fn cato_projections_config_resolves_environment_references() {
    let source = CATO_CONFIG
        .replace("literal-key", "os.environ/CATO_KEY")
        .replace("https://cato.test", "os.environ/CATO_URL");
    let lookup = BTreeMap::from([
        ("CATO_KEY".to_owned(), "env-key".to_owned()),
        ("CATO_URL".to_owned(), "https://env.test".to_owned()),
    ]);
    let settings = resolve(&source, &lookup).unwrap();
    let configured = settings.first().unwrap();
    assert_eq!(
        configured.api_key.as_ref().unwrap().expose_secret(),
        "env-key"
    );
    assert_eq!(configured.api_base.as_deref(), Some("https://env.test"));
}

#[test]
fn cato_projections_config_names_a_missing_environment_reference() {
    let source = CATO_CONFIG.replace("literal-key", "os.environ/CATO_KEY");
    let error = resolve(&source, &BTreeMap::new()).unwrap_err();
    assert!(matches!(
        error,
        ConfigError::MissingEnvironment { variable, path }
        if variable == "CATO_KEY" && path == "guardrails.0.litellm_params.api_key"
    ));
}

#[test]
fn cato_projections_config_reads_a_ca_path_ssl_verify() {
    let source = CATO_CONFIG.replace("ssl_verify: false", "ssl_verify: /etc/ca.pem");
    let settings = resolve(&source, &BTreeMap::new()).unwrap();
    assert_eq!(
        settings.first().unwrap().ssl_verify,
        Some(VerifySetting::Path("/etc/ca.pem".to_owned()))
    );
}
