#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_secret_managers::secret_types::KeyManagementSystem;

use super::{object, text, value};
use crate::config_override_support::environment::{EnvironmentSnapshot, RuntimeEnvironment};
use crate::config_override_support::state::{
    OverrideRuntimeState, SecretManagerControl, SecretManagerInitError, clear_cyberark_state,
    clear_hashicorp_vault_state, restore_cyberark_runtime, snapshot_cyberark_boot_env,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Initialize(KeyManagementSystem),
    Drop,
}

struct Control {
    active: Option<KeyManagementSystem>,
    failing: Option<KeyManagementSystem>,
    calls: Vec<Call>,
}

impl Control {
    const fn new(
        active: Option<KeyManagementSystem>,
        failing: Option<KeyManagementSystem>,
    ) -> Self {
        Self {
            active,
            failing,
            calls: Vec::new(),
        }
    }
}

impl SecretManagerControl for Control {
    fn active_manager(&self) -> Option<KeyManagementSystem> {
        self.active
    }

    fn drop_active_manager(&mut self) {
        self.calls.push(Call::Drop);
        self.active = None;
    }

    fn initialize(&mut self, kind: KeyManagementSystem) -> Result<(), SecretManagerInitError> {
        self.calls.push(Call::Initialize(kind));
        if self.failing == Some(kind) {
            return Err(SecretManagerInitError {
                message: "unreachable".to_owned(),
            });
        }
        self.active = Some(kind);
        Ok(())
    }
}

fn environment(boot: &[(&str, &str)]) -> RuntimeEnvironment {
    RuntimeEnvironment {
        boot: boot
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        overlay: BTreeMap::new(),
    }
}

fn state() -> OverrideRuntimeState {
    OverrideRuntimeState {
        vault_last_config: None,
        cyberark_last_config: None,
        cyberark_boot_env: None,
    }
}

#[test]
fn clear_vault_removes_its_values_and_drops_an_active_vault() {
    let mut environment = environment(&[
        ("HCP_VAULT_ADDR", "https://boot"),
        ("HCP_VAULT_TOKEN", "boot-token"),
        ("CYBERARK_API_BASE", "https://conjur"),
    ]);
    let mut state = state();
    state.vault_last_config = Some(value(r#"{"vault_addr": "https://boot"}"#));
    let mut control = Control::new(Some(KeyManagementSystem::HashicorpVault), None);
    clear_hashicorp_vault_state(&mut environment, &mut state, &mut control);
    assert_eq!(
        environment.snapshot().values,
        BTreeMap::from([("CYBERARK_API_BASE".to_owned(), "https://conjur".to_owned())])
    );
    assert_eq!(control.calls, vec![Call::Drop]);
    assert_eq!(control.active, None);
    assert_eq!(state.vault_last_config, None);
}

#[test]
fn clear_vault_leaves_another_active_manager_alone() {
    let mut environment = environment(&[("HCP_VAULT_ADDR", "https://boot")]);
    let mut state = state();
    state.vault_last_config = Some(value("{}"));
    let mut control = Control::new(Some(KeyManagementSystem::Cyberark), None);
    clear_hashicorp_vault_state(&mut environment, &mut state, &mut control);
    assert!(environment.snapshot().values.is_empty());
    assert!(control.calls.is_empty());
    assert_eq!(control.active, Some(KeyManagementSystem::Cyberark));
    assert_eq!(state.vault_last_config, None);
}

#[test]
fn cyberark_boot_values_are_captured_once() {
    let mut state = state();
    let first = EnvironmentSnapshot {
        values: BTreeMap::from([("CYBERARK_API_BASE".to_owned(), "https://boot".to_owned())]),
    };
    snapshot_cyberark_boot_env(&mut state, &first);
    let second = EnvironmentSnapshot {
        values: BTreeMap::from([
            ("CYBERARK_API_BASE".to_owned(), "https://later".to_owned()),
            ("CYBERARK_ACCOUNT".to_owned(), "acct".to_owned()),
        ]),
    };
    snapshot_cyberark_boot_env(&mut state, &second);
    let boot = state.cyberark_boot_env.unwrap();
    assert_eq!(boot.len(), 8);
    assert_eq!(boot.get("cyberark_api_base"), Some(&text("https://boot")));
    assert_eq!(boot.get("cyberark_account"), Some(&JsonValue::Null));
    assert_eq!(boot.get("client_key"), Some(&JsonValue::Null));
}

#[test]
fn restore_cyberark_applies_its_values_and_initializes_on_api_base() {
    let mut environment = environment(&[
        ("HCP_VAULT_ADDR", "https://vault"),
        ("CYBERARK_ACCOUNT", "old-account"),
    ]);
    let mut state = state();
    state.vault_last_config = Some(value("{}"));
    let mut control = Control::new(None, None);
    restore_cyberark_runtime(
        &mut environment,
        &mut state,
        &mut control,
        &object(r#"{"cyberark_api_base": "https://conjur", "cyberark_api_key": "hunter2"}"#),
    );
    assert_eq!(
        control.calls,
        vec![Call::Initialize(KeyManagementSystem::Cyberark)]
    );
    assert_eq!(control.active, Some(KeyManagementSystem::Cyberark));
    let values = environment.snapshot().values;
    assert_eq!(
        values.get("CYBERARK_API_BASE").map(String::as_str),
        Some("https://conjur")
    );
    assert_eq!(
        values.get("CYBERARK_API_KEY").map(String::as_str),
        Some("hunter2")
    );
    assert!(!values.contains_key("CYBERARK_ACCOUNT"));
    assert_eq!(
        values.get("HCP_VAULT_ADDR").map(String::as_str),
        Some("https://vault")
    );
    assert!(state.vault_last_config.is_some());
}

#[test]
fn restore_falls_back_to_vault_after_dropping_an_active_cyberark() {
    let mut environment = environment(&[("HCP_VAULT_ADDR", "https://vault")]);
    let mut state = state();
    state.vault_last_config = Some(value("{}"));
    let mut control = Control::new(
        Some(KeyManagementSystem::Cyberark),
        Some(KeyManagementSystem::Cyberark),
    );
    restore_cyberark_runtime(
        &mut environment,
        &mut state,
        &mut control,
        &object(r#"{"cyberark_api_base": "https://conjur"}"#),
    );
    assert_eq!(
        control.calls,
        vec![
            Call::Initialize(KeyManagementSystem::Cyberark),
            Call::Drop,
            Call::Initialize(KeyManagementSystem::HashicorpVault),
        ]
    );
    assert_eq!(control.active, Some(KeyManagementSystem::HashicorpVault));
    assert_eq!(state.vault_last_config, None);
}

#[test]
fn restore_never_reinitializes_vault_without_an_active_cyberark() {
    let mut environment = environment(&[("HCP_VAULT_ADDR", "https://vault")]);
    let mut state = state();
    state.vault_last_config = Some(value("{}"));
    let mut control = Control::new(None, Some(KeyManagementSystem::Cyberark));
    restore_cyberark_runtime(
        &mut environment,
        &mut state,
        &mut control,
        &object(r#"{"cyberark_api_base": "https://conjur"}"#),
    );
    assert_eq!(
        control.calls,
        vec![Call::Initialize(KeyManagementSystem::Cyberark)]
    );
    assert_eq!(control.active, None);
    assert!(state.vault_last_config.is_some());
}

#[test]
fn restore_drops_cyberark_without_a_vault_address_to_return_to() {
    let mut environment = environment(&[]);
    let mut state = state();
    state.vault_last_config = Some(value("{}"));
    let mut control = Control::new(Some(KeyManagementSystem::Cyberark), None);
    restore_cyberark_runtime(
        &mut environment,
        &mut state,
        &mut control,
        &object(r#"{"cyberark_account": "acct"}"#),
    );
    assert_eq!(control.calls, vec![Call::Drop]);
    assert_eq!(control.active, None);
    assert_eq!(state.vault_last_config, None);
}

#[test]
fn restore_skips_vault_when_its_address_is_empty() {
    let mut environment = environment(&[("HCP_VAULT_ADDR", "")]);
    let mut state = state();
    state.vault_last_config = Some(value("{}"));
    let mut control = Control::new(Some(KeyManagementSystem::Cyberark), None);
    restore_cyberark_runtime(
        &mut environment,
        &mut state,
        &mut control,
        &object(r#"{"cyberark_account": "acct"}"#),
    );
    assert_eq!(control.calls, vec![Call::Drop]);
    assert_eq!(control.active, None);
    assert_eq!(state.vault_last_config, None);
}

#[test]
fn clear_cyberark_restores_boot_values_and_clears_its_marker() {
    let mut environment = environment(&[]);
    let mut state = state();
    state.cyberark_last_config = Some(value(r#"{"cyberark_api_base": "https://stored"}"#));
    state.cyberark_boot_env = Some(object(
        r#"{
            "cyberark_api_base": "https://boot",
            "cyberark_account": "acct",
            "client_key": null
        }"#,
    ));
    let mut control = Control::new(None, None);
    clear_cyberark_state(&mut environment, &mut state, &mut control);
    assert_eq!(
        control.calls,
        vec![Call::Initialize(KeyManagementSystem::Cyberark)]
    );
    assert_eq!(
        environment.snapshot().values,
        BTreeMap::from([
            ("CYBERARK_API_BASE".to_owned(), "https://boot".to_owned()),
            ("CYBERARK_ACCOUNT".to_owned(), "acct".to_owned()),
        ])
    );
    assert_eq!(state.cyberark_last_config, None);
    assert!(state.cyberark_boot_env.is_some());
}

#[test]
fn clear_cyberark_without_boot_values_hands_back_to_vault() {
    let mut environment = environment(&[("HCP_VAULT_ADDR", "https://vault")]);
    let mut state = state();
    state.cyberark_last_config = Some(value("{}"));
    let mut control = Control::new(Some(KeyManagementSystem::Cyberark), None);
    clear_cyberark_state(&mut environment, &mut state, &mut control);
    assert_eq!(
        control.calls,
        vec![
            Call::Drop,
            Call::Initialize(KeyManagementSystem::HashicorpVault)
        ]
    );
    assert_eq!(control.active, Some(KeyManagementSystem::HashicorpVault));
    assert_eq!(state.cyberark_last_config, None);
    assert_eq!(state.cyberark_boot_env, None);
}
