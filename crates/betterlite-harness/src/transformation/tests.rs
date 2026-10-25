#![cfg(test)]
use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};

use crate::sandbox::SandboxCommand;
use crate::transformation::{HarnessSessionSetup, HarnessTurnRequest};

#[test]
fn session_and_turn_debug_redact_endpoint_tokens() {
    let token = "test-endpoint-token";
    let key = "ANTHROPIC_AUTH_TOKEN";
    let mut setup = HarnessSessionSetup::default();
    setup.env.insert(key.to_owned(), SecretString::from(token));
    let request = HarnessTurnRequest {
        argv: vec!["claude".to_owned()],
        env: setup.env.clone(),
        stdin: String::new(),
        cwd: None,
    };
    assert!(!format!("{setup:?}").contains(token));
    assert!(!format!("{request:?}").contains(token));
    assert_eq!(
        request.env.get(key).map(ExposeSecret::expose_secret),
        Some(token)
    );
    let command = SandboxCommand {
        argv: vec!["claude".to_owned()],
        env: BTreeMap::from([("ANTHROPIC_AUTH_TOKEN".to_owned(), SecretString::from(token))]),
        cwd: None,
    };
    assert!(!format!("{command:?}").contains(token));
}
