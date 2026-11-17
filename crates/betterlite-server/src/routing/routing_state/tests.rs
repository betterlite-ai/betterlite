#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_router::generalization_integration::bounds::MAX_RULES_BYTES;
use betterlite_router::router_core::services::RouteServices;
use tempfile::NamedTempFile;
use tracing::subscriber::DefaultGuard;
use tracing::subscriber::set_default;

use super::RoutingState;

const ACME: &str = "fallback_generalizations:\n  rules:\n    - pattern: \"^acme-\"\n      model_info:\n        litellm_provider: acme\n";
const BETA: &str = "fallback_generalizations:\n  rules:\n    - pattern: \"^beta-\"\n      model_info:\n        litellm_provider: beta\n";

fn config(source: &str) -> Config {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap()
}

fn capture_log() -> (NamedTempFile, DefaultGuard) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    (log, guard)
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let (log, guard) = capture_log();
    let result = action();
    drop(guard);
    (result, fs::read_to_string(log.path()).unwrap())
}

#[test]
fn the_loaded_config_installs_its_rules() {
    let state = RoutingState::new(&config(ACME));
    let installed = state.model_generalizations().unwrap();
    assert_eq!(installed.routing("acme-1").answer.as_deref(), Some("acme"));
}

#[test]
fn a_reload_replaces_the_rules_the_server_serves() {
    let mut state = RoutingState::new(&config(ACME));
    state.reload(&config(BETA)).unwrap();
    let installed = state.model_generalizations().unwrap();
    assert_eq!(installed.routing("beta-1").answer.as_deref(), Some("beta"));
    assert_eq!(installed.routing("acme-1").answer, None);
}

#[test]
fn a_config_without_rules_installs_the_shipped_cost_maps_rules() {
    let state = RoutingState::new(&config("model_list: []\n"));
    let installed = state.model_generalizations().unwrap();
    assert_eq!(
        installed.routing("claude-sonnet-4").answer.as_deref(),
        Some("anthropic")
    );
    assert_eq!(
        installed
            .routing("anthropic.claude-sonnet-4")
            .answer
            .as_deref(),
        Some("bedrock")
    );
}

#[test]
fn a_reload_to_a_config_without_rules_restores_the_shipped_rules() {
    let mut state = RoutingState::new(&config(ACME));
    state.reload(&config("model_list: []\n")).unwrap();
    let installed = state.model_generalizations().unwrap();
    assert_eq!(installed.routing("acme-1").answer, None);
    assert_eq!(
        installed.routing("claude-sonnet-4").answer.as_deref(),
        Some("anthropic")
    );
}

#[test]
fn a_reload_that_cannot_install_keeps_the_rules_the_server_serves() {
    let mut state = RoutingState::new(&config(ACME));
    assert!(state.reload(&config(&oversized_rules())).is_err());
    let installed = state.model_generalizations().unwrap();
    assert_eq!(installed.routing("acme-1").answer.as_deref(), Some("acme"));
}

fn oversized_rules() -> String {
    let pattern = "a".repeat(MAX_RULES_BYTES);
    format!(
        "fallback_generalizations:\n  rules:\n    - pattern: \"{pattern}\"\n      model_info: {{}}\n"
    )
}

#[test]
fn an_uninstallable_rule_document_is_logged_and_serves_no_rules() {
    let document = oversized_rules();
    let (state, log) = logged(|| RoutingState::new(&config(&document)));
    let installed = state.model_generalizations().unwrap();
    assert!(installed.routing("acme-1").answer.is_none());
    assert!(installed.routing("claude-sonnet-4").answer.is_none());
    assert!(log.contains("unreadable"), "{log}");
}
