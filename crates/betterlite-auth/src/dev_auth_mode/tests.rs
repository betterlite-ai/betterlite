#![cfg(test)]

use secrecy::SecretString;

use super::{AuthModeFlags, AuthModeSwitch, is_no_auth_dev_mode};

#[test]
fn default_flags_are_all_off() {
    let flags = AuthModeFlags::default();
    assert!(!flags.enable_jwt_auth.0);
    assert!(!flags.enable_oauth2_auth.0);
    assert!(!flags.enable_oauth2_proxy_auth.0);
}

#[test]
fn no_master_key_with_every_switch_off_is_dev_mode() {
    assert!(is_no_auth_dev_mode(None, &AuthModeFlags::default()));
}

#[test]
fn a_master_key_is_not_dev_mode() {
    let master_key = SecretString::from("sk-123");
    assert!(!is_no_auth_dev_mode(
        Some(&master_key),
        &AuthModeFlags::default()
    ));
}

#[test]
fn an_empty_master_key_is_not_dev_mode() {
    let master_key = SecretString::from("");
    assert!(!is_no_auth_dev_mode(
        Some(&master_key),
        &AuthModeFlags::default()
    ));
}

#[test]
fn jwt_auth_is_not_dev_mode() {
    let flags = AuthModeFlags {
        enable_jwt_auth: AuthModeSwitch(true),
        enable_oauth2_auth: AuthModeSwitch(false),
        enable_oauth2_proxy_auth: AuthModeSwitch(false),
    };
    assert!(!is_no_auth_dev_mode(None, &flags));
}

#[test]
fn oauth2_auth_is_not_dev_mode() {
    let flags = AuthModeFlags {
        enable_jwt_auth: AuthModeSwitch(false),
        enable_oauth2_auth: AuthModeSwitch(true),
        enable_oauth2_proxy_auth: AuthModeSwitch(false),
    };
    assert!(!is_no_auth_dev_mode(None, &flags));
}

#[test]
fn oauth2_proxy_auth_is_not_dev_mode() {
    let flags = AuthModeFlags {
        enable_jwt_auth: AuthModeSwitch(false),
        enable_oauth2_auth: AuthModeSwitch(false),
        enable_oauth2_proxy_auth: AuthModeSwitch(true),
    };
    assert!(!is_no_auth_dev_mode(None, &flags));
}
