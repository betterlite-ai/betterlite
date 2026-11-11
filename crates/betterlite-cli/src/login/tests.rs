#![cfg(test)]

mod support;

use std::fs;

use betterlite_config::sections::settings::JsonValue;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::claude_settings::SettingsDocument;
use crate::claude_settings::error::ClaudeSettingsError;
use crate::credential_lifecycle::outcome::SecretSave;
use crate::login::error::{FlowError, LoginError, PreflightError};
use crate::login::finish::LoginFinished;
use crate::login::{Interrupts as _, LoginArgs, NeverInterrupted};

use support::{World, response, run, secret, start_body, world};

fn finish_stored(
    world: &mut World,
    stored: &SecretSave,
    config_claude: bool,
) -> (Result<(), FlowError>, String) {
    let api_key = secret("sk-finish-token-with-more");
    let finished = LoginFinished {
        base_url: support::SERVER,
        api_key: &api_key,
        config_claude,
        stored,
    };
    support::finish(world, finished)
}

fn assert_pkce_record(world: &World) {
    let record = support::stored(world).expect("the token file");
    assert_eq!(record.base_url, support::SERVER);
    assert_eq!(record.user_id, "user-7");
    assert_eq!(record.user_email, "unknown");
    assert_eq!(record.user_role, "cli");
    assert_eq!(record.auth_header_name, "Authorization");
    assert_eq!(
        record.expires_at.map(f64::to_bits),
        Some(1_700_003_600.0_f64.to_bits())
    );
    assert_eq!(record.client_id.as_deref(), Some("client-1"));
    assert!(record.key.is_none());
    let stored = support::stored_secret(world).expect("the saved secret");
    assert_eq!(
        stored.key.as_ref().expect("key").expose_secret(),
        "sk-access"
    );
    assert_eq!(
        stored
            .refresh_token
            .as_ref()
            .expect("refresh")
            .expose_secret(),
        "refresh-1"
    );
    assert_eq!(stored.timestamp.to_bits(), 1_700_000_000.0_f64.to_bits());
}

fn assert_sso_record(world: &World) {
    let record = support::stored(world).expect("the token file");
    assert_eq!(record.base_url, support::SERVER);
    assert_eq!(record.user_id, "user-9");
    assert_eq!(record.user_email, "unknown");
    assert_eq!(record.user_role, "cli");
    assert_eq!(record.auth_header_name, "Authorization");
    assert_eq!(record.jwt_token.expose_secret(), "");
    assert!(record.key.is_none());
    assert!(record.expires_at.is_none());
    assert!(record.refresh_token.is_none());
    let stored = support::stored_secret(world).expect("the saved secret");
    assert_eq!(stored.key.as_ref().expect("key").expose_secret(), "sk-sso");
    assert_eq!(stored.jwt_token.expose_secret(), "");
    assert!(stored.refresh_token.is_none());
    assert_eq!(stored.timestamp.to_bits(), 1_700_000_000.0_f64.to_bits());
}

#[test]
fn login_dispatch_refuses_before_the_flow_when_claude_is_owned() {
    let mut world = world(Vec::new());
    let backup = world
        .home
        .path()
        .join(".litellm")
        .join("claude_settings_backup.json");
    fs::create_dir_all(backup.parent().expect("the backup directory"))
        .expect("creates the backup directory");
    fs::write(&backup, "{}").expect("writes the backup marker");
    let (result, out) = run(
        &mut world,
        LoginArgs {
            config_claude: true,
            pkce: true,
        },
    );
    let error = result.expect_err("refuses to log in");
    assert!(matches!(
        &error,
        LoginError::NotLoggingIn {
            source: PreflightError::Settings(ClaudeSettingsError::Owned { .. })
        }
    ));
    assert!(
        error
            .to_string()
            .starts_with("Cannot configure Claude Code, so not logging in: ")
    );
    assert_eq!(out, "");
    assert!(world.http.requests().is_empty());
    assert!(
        world
            .browser_calls
            .lock()
            .expect("the browser log")
            .is_empty()
    );
}

#[test]
fn login_dispatch_refuses_without_a_home_when_claude_configuration_is_requested() {
    let mut world = world(Vec::new());
    world.environment = support::FakeEnvironment::homeless();
    let (result, _out) = run(
        &mut world,
        LoginArgs {
            config_claude: true,
            pkce: false,
        },
    );
    let error = result.expect_err("refuses to log in");
    assert!(matches!(
        &error,
        LoginError::NotLoggingIn {
            source: PreflightError::NoHome
        }
    ));
    assert!(world.http.requests().is_empty());
}

#[test]
fn login_dispatch_runs_the_pkce_flow_with_the_pkce_flag() {
    let mut world = world(vec![(support::DISCOVERY, response(500, ""))]);
    let (result, out) = run(
        &mut world,
        LoginArgs {
            config_claude: false,
            pkce: true,
        },
    );
    result.expect("reports the failure as a message");
    assert_eq!(
        out,
        "Authentication failed: discovery returned status 500\n"
    );
    assert_eq!(
        world.http.requests(),
        vec![format!("{}{}", support::SERVER, support::DISCOVERY)]
    );
}

#[test]
fn login_dispatch_runs_the_sso_flow_by_default() {
    let mut world = world(vec![(support::START, response(404, ""))]);
    let (result, out) = run(&mut world, LoginArgs::default());
    result.expect("reports the failure as a message");
    assert!(out.contains("Authentication failed: POST "));
    assert!(out.contains("/sso/cli/start returned HTTP 404"));
    assert_eq!(
        world.http.requests(),
        vec![format!("{}{}", support::SERVER, support::START)]
    );
}

#[test]
fn login_dispatch_reports_a_cancellation_as_a_message() {
    let mut world = world(vec![(support::START, response(404, ""))]);
    world.interrupt = support::FakeInterrupt(true);
    let (result, out) = run(&mut world, LoginArgs::default());
    result.expect("treats the cancellation as a plain message");
    assert_eq!(out, "\nAuthentication cancelled by user.\n");
}

#[test]
fn login_dispatch_never_reports_an_interruption_by_default() {
    assert!(!NeverInterrupted.interrupted());
}

#[test]
fn login_dispatch_prints_the_failure_when_the_flow_is_not_interrupted() {
    let mut world = world(vec![(support::START, response(404, ""))]);
    let (result, out) = run(&mut world, LoginArgs::default());
    result.expect("reports the failure as a message");
    assert!(out.starts_with("Authentication failed: "));
    assert!(!out.contains("cancelled"));
}

#[test]
fn pkce_login_prints_the_failure_reason_and_saves_nothing() {
    let mut world = world(vec![(support::DISCOVERY, response(500, ""))]);
    let (result, out) = run(
        &mut world,
        LoginArgs {
            config_claude: false,
            pkce: true,
        },
    );
    result.expect("reports the failure as a message");
    assert_eq!(
        out,
        "Authentication failed: discovery returned status 500\n"
    );
    assert!(support::stored(&world).is_none());
    assert!(world.vault.writes.is_empty());
    assert!(
        world
            .browser_calls
            .lock()
            .expect("the browser log")
            .is_empty()
    );
}

#[test]
fn pkce_login_saves_the_record_and_finishes_the_login() {
    let mut world = world(support::pkce_routes());
    world.delivers_callback = true;
    let (result, out) = run(
        &mut world,
        LoginArgs {
            config_claude: false,
            pkce: true,
        },
    );
    result.expect("signs in");
    assert!(out.contains("Opening browser to: https://gateway.example/authorize?"));
    assert!(out.contains("Approve the sign-in in your browser. Waiting..."));
    assert!(out.contains("\nLogin successful!\nJWT Token: sk-access...\n"));
    assert!(out.contains("Credential stored in your OS keychain."));
    assert!(out.contains("You can now use the CLI without specifying --api-key"));
    assert!(out.contains("\n============================================================\n"));
    assert!(out.contains("Available commands:"));
    assert_pkce_record(&world);
}

#[test]
fn pkce_login_revokes_the_previous_login_through_the_stored_token_replacement() {
    let mut world = world(support::pkce_routes());
    world.delivers_callback = true;
    support::seed(&*world.files, &mut world.vault, support::record());
    let (result, _out) = run(
        &mut world,
        LoginArgs {
            config_claude: false,
            pkce: true,
        },
    );
    result.expect("signs in");
    let requests = world.http.requests();
    assert!(requests.contains(&String::from(support::REVOCATION_ENDPOINT)));
    let stored = support::stored_secret(&world).expect("the new secret");
    assert_eq!(
        stored.key.as_ref().expect("key").expose_secret(),
        "sk-access"
    );
}

#[test]
fn sso_login_saves_litellms_placeholder_record_fields() {
    let mut world = world(vec![
        (support::START, response(200, &start_body(false))),
        (support::POLL, response(200, &support::ready_body())),
    ]);
    let (result, out) = run(&mut world, LoginArgs::default());
    result.expect("signs in");
    assert_sso_record(&world);
    assert!(out.contains("Login successful!"));
    assert!(out.contains("JWT Token: sk-sso..."));
}

#[test]
fn sso_login_stores_cli_user_when_the_proxy_returns_no_user_id() {
    let mut world = world(vec![
        (support::START, response(200, &start_body(false))),
        (
            support::POLL,
            response(200, r#"{"status":"ready","key":"sk-sso"}"#),
        ),
    ]);
    let (result, _out) = run(&mut world, LoginArgs::default());
    result.expect("signs in");
    let record = support::stored(&world).expect("the token file");
    assert_eq!(record.user_id, "cli-user");
}

#[test]
fn sso_login_opens_the_generate_url_with_the_source_and_login_id() {
    let mut world = world(vec![
        (support::START, response(200, &start_body(false))),
        (support::POLL, response(200, &support::ready_body())),
    ]);
    let (result, out) = run(&mut world, LoginArgs::default());
    result.expect("signs in");
    let url = format!(
        "{}/sso/key/generate?source=litellm-cli&key=login-1",
        support::SERVER
    );
    assert_eq!(
        *world.browser_calls.lock().expect("the browser log"),
        vec![url.clone()]
    );
    assert!(out.starts_with(&format!("Opening browser to: {url}\n")));
    assert!(out.contains("Please complete the SSO authentication in your browser...\n"));
    assert!(out.contains("Verification code: code-1\n"));
    assert!(out.contains("Session ID: login-1\n"));
    assert!(out.contains("Waiting for authentication...\n"));
}

#[test]
fn sso_login_prefills_the_user_code_when_the_proxy_offers_it() {
    let mut world = world(vec![
        (support::START, response(200, &start_body(true))),
        (support::POLL, response(200, &support::ready_body())),
    ]);
    let (result, out) = run(&mut world, LoginArgs::default());
    result.expect("signs in");
    assert_eq!(
        *world.browser_calls.lock().expect("the browser log"),
        vec![format!(
            "{}/sso/key/generate?source=litellm-cli&key=login-1&user_code=code-1",
            support::SERVER
        )]
    );
    assert!(
        out.contains("Verification code: code-1 (pre-filled in the browser, check it matches)\n")
    );
}

#[test]
fn sso_login_prints_the_timeout_message_and_saves_nothing() {
    let mut world = world(vec![
        (support::START, response(200, &start_body(false))),
        (support::POLL, response(200, r#"{"status":"pending"}"#)),
    ]);
    let (result, out) = run(&mut world, LoginArgs::default());
    result.expect("reports the timeout as a message");
    assert!(out.contains("Authentication timed out. Please try again.\n"));
    assert!(out.contains("The proxy never reported the browser sign-in as finished."));
    assert!(support::stored(&world).is_none());
    assert!(world.vault.writes.is_empty());
    assert!(
        !world
            .sleeper
            .slept
            .lock()
            .expect("the sleeper log")
            .is_empty()
    );
}

#[test]
fn finish_login_prints_the_success_message_the_prefix_and_the_storage_notice() {
    let mut world = world(Vec::new());
    let (result, out) = finish_stored(&mut world, &SecretSave::SecretStored, false);
    result.expect("finishes the login");
    assert!(out.starts_with("\nLogin successful!\nJWT Token: sk-finish-token-with...\n"));
    assert!(out.contains("Credential stored in your OS keychain.\n"));
    assert!(out.contains("You can now use the CLI without specifying --api-key\n"));
    assert!(out.contains("\n============================================================\n"));
    assert!(out.contains("Available commands:\n"));
}

#[test]
fn finish_login_stops_before_the_usable_line_for_a_credential_not_saved() {
    let mut world = world(Vec::new());
    let (result, out) = finish_stored(&mut world, &support::credential_not_saved(), false);
    result.expect("finishes the login");
    assert!(out.contains("Signed in, but the credential could not be saved"));
    assert!(!out.contains("You can now use the CLI"));
    assert!(!out.contains("Available commands:"));
}

#[test]
fn finish_login_stops_before_the_usable_line_for_a_credential_not_recorded() {
    let mut world = world(Vec::new());
    let (result, out) = finish_stored(&mut world, &support::credential_not_recorded(), false);
    result.expect("finishes the login");
    assert!(out.contains("Signed in, and the credential is in your OS keychain"));
    assert!(!out.contains("You can now use the CLI"));
    assert!(!out.contains("Available commands:"));
}

#[rstest]
#[case::keyring_disabled(|| SecretSave::KeyringDisabled)]
#[case::keyring_not_installed(|| SecretSave::KeyringNotInstalled)]
#[case::keyring_unreachable(|| SecretSave::KeyringUnreachable)]
#[case::keyring_discards_writes(|| SecretSave::KeyringDiscardsWrites)]
fn finish_login_continues_for_every_keychain_fallback(#[case] stored: fn() -> SecretSave) {
    let mut world = world(Vec::new());
    let (result, out) = finish_stored(&mut world, &stored(), false);
    result.expect("finishes the login");
    assert!(out.contains("You can now use the CLI without specifying --api-key"));
    assert!(out.contains("Available commands:"));
}

#[test]
fn finish_login_configures_claude_code_when_requested() {
    let mut world = world(Vec::new());
    let (result, out) = finish_stored(&mut world, &support::credential_not_saved(), true);
    result.expect("finishes the login");
    assert!(out.contains("Configured Claude Code:"));
    assert!(out.contains(
        "Claude Code was configured with this key even though the CLI itself could not keep it."
    ));
    assert!(!out.contains("You can now use the CLI"));
    let settings = fs::read_to_string(world.home.path().join(".claude").join("settings.json"))
        .expect("the settings file");
    let document: SettingsDocument =
        serde_json::from_str(&settings).expect("the settings document");
    let Some(JsonValue::Object(env)) = document.get("env") else {
        panic!("env is an object");
    };
    assert_eq!(
        env.get("ANTHROPIC_AUTH_TOKEN").and_then(JsonValue::as_str),
        Some("sk-finish-token-with-more")
    );
    assert_eq!(
        env.get("ANTHROPIC_BASE_URL").and_then(JsonValue::as_str),
        Some("https://gateway.example")
    );
}

#[test]
fn finish_login_reports_a_configuration_failure_as_an_error() {
    let mut world = world(Vec::new());
    let settings_directory = world.home.path().join(".claude");
    fs::create_dir_all(&settings_directory).expect("creates the settings directory");
    fs::write(settings_directory.join("settings.json"), "[]").expect("writes the bad settings");
    let (result, out) = finish_stored(&mut world, &SecretSave::SecretStored, true);
    let error = result.expect_err("reports the configuration failure");
    assert!(matches!(error, FlowError::Configure(_)));
    assert!(
        error
            .to_string()
            .starts_with("Logged in, but could not configure Claude Code: ")
    );
    assert!(!out.contains("Claude Code was configured"));
}
