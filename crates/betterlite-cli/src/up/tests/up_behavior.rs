use secrecy::SecretString;

use super::support::{World, pkce_record, record};
use crate::up::command::run_up;
use crate::up::error::UpError;
use crate::up::login::ensure_fresh_login;

const FRESH: f64 = 4_000_000_000.0;

#[test]
fn ensure_fresh_login_reports_the_login_command_without_a_token() {
    let mut world = World::new(false);
    let mut ports = world.ports();
    let error = ensure_fresh_login(&mut ports).expect_err("no stored login");
    drop(ports);
    assert_eq!(
        error.to_string(),
        "No fresh LiteLLM login found for this proxy. Run `lite login` first."
    );
    assert_eq!(world.login_count(), 0);
}

#[test]
fn ensure_fresh_login_starts_login_on_a_terminal() {
    let mut world = World::new(true);
    world.login.token = Some(record("sk-login", FRESH, Some(FRESH)));
    let mut ports = world.ports();
    ensure_fresh_login(&mut ports).expect("the login supplies a fresh token");
    drop(ports);
    assert_eq!(world.login_count(), 1);
    assert_eq!(
        world.text(),
        "No fresh LiteLLM login found for this proxy; starting login...\n"
    );
}

#[test]
fn ensure_fresh_login_rejects_a_stale_token() {
    let mut world = World::new(false);
    world.seed_token(record("sk-old", 1_000.0, None));
    let mut ports = world.ports();
    let error = ensure_fresh_login(&mut ports).expect_err("the token is stale");
    drop(ports);
    assert!(matches!(error, UpError::NoFreshLogin { .. }));
    assert_eq!(world.login_count(), 0);
}

#[test]
fn ensure_fresh_login_accepts_a_fresh_token() {
    let mut world = World::new(false);
    world.seed_token(record("sk-live", 1_000.0, Some(FRESH)));
    let mut ports = world.ports();
    ensure_fresh_login(&mut ports).expect("a fresh token needs no login");
    drop(ports);
    assert_eq!(world.login_count(), 0);
}

#[test]
fn ensure_fresh_login_points_a_pkce_record_at_pkce_login() {
    let mut world = World::new(false);
    world.seed_token(pkce_record("sk-pkce", 1_000.0));
    let mut ports = world.ports();
    let error = ensure_fresh_login(&mut ports).expect_err("the token is stale");
    assert_eq!(
        error.to_string(),
        "No fresh LiteLLM login found for this proxy. Run `lite login --pkce` first."
    );
}

#[test]
fn ensure_fresh_login_ignores_an_explicit_api_key() {
    let mut world = World::new(false);
    world.context.api_key = Some(SecretString::from("sk-explicit".to_owned()));
    let mut ports = world.ports();
    let error = ensure_fresh_login(&mut ports).expect_err("an explicit key is not a login");
    assert!(matches!(error, UpError::NoFreshLogin { .. }));
}

#[test]
fn up_verifies_the_key_with_the_proxy_before_patching() {
    let mut world = World::new(false);
    world.seed_token(record("sk-live", 1_000.0, Some(FRESH)));
    world.context.api_key = Some(SecretString::from("sk-live".to_owned()));
    world.probe.status = 401;
    let mut ports = world.ports();
    let error = run_up(&mut ports).expect_err("the proxy rejects the key");
    drop(ports);
    assert_eq!(
        error.to_string(),
        "LiteLLM rejected your key (HTTP 401). Run `lite login` to refresh it, or pass a valid --api-key."
    );
    assert!(!world.settings_path().exists());
    assert!(!world.backup_path().exists());
    let probe = world.probe_record().expect("the key was probed");
    assert_eq!(probe.url, "http://gateway.test:4000/v1/models");
    assert_eq!(probe.timeout, 10);
    assert_eq!(probe.authorization, "Bearer sk-live");
}

#[test]
fn up_refuses_while_a_backup_exists() {
    let mut world = World::new(false);
    world.seed_token(record("sk-live", 1_000.0, Some(FRESH)));
    world.context.api_key = Some(SecretString::from("sk-live".to_owned()));
    world.seed(
        &world.backup_path(),
        r#"{"existed": false, "content": null}"#,
    );
    let mut ports = world.ports();
    let error = run_up(&mut ports).expect_err("a backup holds the settings");
    drop(ports);
    let backup = world.backup_path().display();
    assert_eq!(
        error.to_string(),
        format!(
            "{backup} already exists -- `lite up` looks like it's already running (or crashed without cleanup). Run `lite down` first."
        )
    );
    assert!(!world.settings_path().exists());
}
