#![cfg(test)]
use std::collections::BTreeMap;

use crate::loader::ConfigLoader;

use super::Settings;

fn load(source: &str, environment: BTreeMap<String, String>) -> Settings {
    ConfigLoader {
        lookup: environment,
    }
    .load(source)
    .unwrap()
    .sections
    .mcp_bearer_credentials
}

#[test]
fn absent_when_general_settings_has_no_signing_block() {
    let settings = load("{}", BTreeMap::new());
    assert!(matches!(settings, Settings::Absent));
}

#[test]
fn invalid_when_signing_block_is_not_a_mapping() {
    let source = "
general_settings:
  mcp_session_token_signing: \"not-a-mapping\"
";
    let settings = load(source, BTreeMap::new());
    assert!(matches!(settings, Settings::Invalid(_)));
}

#[test]
fn invalid_when_algorithm_is_not_rs256() {
    let source = "
general_settings:
  mcp_session_token_signing:
    algorithm: HS256
    kid: key-1
    private_key: inline-pem
";
    let settings = load(source, BTreeMap::new());
    assert!(matches!(settings, Settings::Invalid(_)));
}

#[test]
fn invalid_when_kid_is_empty() {
    let source = "
general_settings:
  mcp_session_token_signing:
    algorithm: RS256
    kid: \"\"
    private_key: inline-pem
";
    let settings = load(source, BTreeMap::new());
    assert!(matches!(settings, Settings::Invalid(_)));
}

#[test]
fn invalid_when_an_unknown_field_is_present() {
    let source = "
general_settings:
  mcp_session_token_signing:
    algorithm: RS256
    kid: key-1
    private_key: inline-pem
    unknown_field: oops
";
    let settings = load(source, BTreeMap::new());
    assert!(matches!(settings, Settings::Invalid(_)));
}

#[test]
fn invalid_when_private_key_environment_reference_is_unresolved() {
    let source = "
general_settings:
  mcp_session_token_signing:
    algorithm: RS256
    kid: key-1
    private_key: os.environ/MCP_SIGNING_PRIVATE_KEY
";
    let settings = load(source, BTreeMap::new());
    assert!(matches!(settings, Settings::Invalid(_)));
}

#[test]
fn invalid_when_a_previous_public_key_environment_reference_is_unresolved() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "MCP_SIGNING_PRIVATE_KEY".to_owned(),
        "inline-pem".to_owned(),
    );
    let source = "
general_settings:
  mcp_session_token_signing:
    algorithm: RS256
    kid: key-1
    private_key: os.environ/MCP_SIGNING_PRIVATE_KEY
    previous_public_keys:
      - kid: key-0
        public_key: os.environ/MCP_SIGNING_PREVIOUS_KEY
";
    let settings = load(source, environment);
    match settings {
        Settings::Invalid(detail) => assert!(detail.contains("key-0")),
        Settings::Absent | Settings::Resolved(_) => panic!("expected Invalid, got {settings:?}"),
    }
}

#[test]
fn resolves_inline_private_key_and_previous_public_keys() {
    let source = "
general_settings:
  mcp_session_token_signing:
    algorithm: RS256
    kid: key-1
    private_key: inline-private-pem
    previous_public_keys:
      - kid: key-0
        public_key: inline-public-pem
";
    let settings = load(source, BTreeMap::new());
    match settings {
        Settings::Resolved(resolved) => {
            assert_eq!(resolved.kid, "key-1");
            assert_eq!(resolved.previous_public_keys.len(), 1);
            assert_eq!(resolved.previous_public_keys[0].kid, "key-0");
        }
        Settings::Absent | Settings::Invalid(_) => {
            panic!("expected Resolved, got {settings:?}")
        }
    }
}

#[test]
fn resolves_private_key_and_previous_public_key_from_environment() {
    let mut environment = BTreeMap::new();
    environment.insert(
        "MCP_SIGNING_PRIVATE_KEY".to_owned(),
        "env-private-pem".to_owned(),
    );
    environment.insert(
        "MCP_SIGNING_PREVIOUS_KEY".to_owned(),
        "env-public-pem".to_owned(),
    );
    let source = "
general_settings:
  mcp_session_token_signing:
    algorithm: RS256
    kid: key-1
    private_key: os.environ/MCP_SIGNING_PRIVATE_KEY
    previous_public_keys:
      - kid: key-0
        public_key: os.environ/MCP_SIGNING_PREVIOUS_KEY
";
    let settings = load(source, environment);
    match &settings {
        Settings::Resolved(resolved) => {
            use secrecy::ExposeSecret as _;
            assert_eq!(resolved.private_key_pem.expose_secret(), "env-private-pem");
            assert_eq!(
                resolved.previous_public_keys[0]
                    .public_key_pem
                    .expose_secret(),
                "env-public-pem"
            );
        }
        Settings::Absent | Settings::Invalid(_) => {
            panic!("expected Resolved, got {settings:?}")
        }
    }
}
