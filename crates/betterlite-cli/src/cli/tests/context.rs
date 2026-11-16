use std::fs;
use std::path::Path;
use std::sync::Arc;

use betterlite_logging::private_json::files::PrivateFiles;
use secrecy::{ExposeSecret, SecretString};
use tokio::runtime::Builder;

use crate::cli::Failure;
use crate::cli::context::{self, GlobalArgs, Request};
use crate::cli::session::CliClock;
use crate::cli::tests::support::{Captured, FakeEnvironment, FakeVault, WorkingVault, channel};
use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::ManagedTokenFile;
use crate::credential_lifecycle::outcome::SecretSave;
use crate::credential_lifecycle::save::save_cli_token;
use crate::token_records::file::token_file_path;
use crate::token_records::record::CliTokenRecord;

fn write_config(home: &Path, contents: &str) {
    let directory = home.join(".litellm");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("config.json"), contents).unwrap();
}

fn store_base_url(home: &Path, base_url: &str) {
    write_config(home, &format!("{{\"base_url\":\"{base_url}\"}}"));
}

fn flags(base_url: Option<&str>, api_key: Option<&str>) -> GlobalArgs {
    GlobalArgs {
        base_url: base_url.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
    }
}

fn resolve(
    environment: &FakeEnvironment,
    flags: GlobalArgs,
    subcommand: Option<&str>,
) -> Result<CliContext, Failure> {
    let mut vault = FakeVault;
    let stderr = Captured::default();
    context::resolve_for_run(&mut Request {
        environment,
        globals: flags,
        subcommand,
        vault: &mut vault,
        clock: Arc::new(CliClock),
        token_path: None,
        stderr: channel(Box::new(stderr)),
    })
}

#[test]
fn nothing_configured_falls_back_to_localhost() {
    let context = resolve(
        &FakeEnvironment::default(),
        flags(None, None),
        Some("models"),
    )
    .unwrap();
    assert_eq!(context.base_url, "http://localhost:4000");
    assert!(!context.base_url_explicit);
    assert!(context.api_key.is_none());
}

#[test]
fn flag_beats_environment_and_stored_config() {
    let home = tempfile::tempdir().unwrap();
    store_base_url(home.path(), "https://stored.example");
    let environment = FakeEnvironment::with_home(home.path().to_path_buf())
        .with_variable("LITELLM_PROXY_URL", "https://environment.example");
    let context = resolve(
        &environment,
        flags(Some("https://flag.example"), None),
        Some("models"),
    )
    .unwrap();
    assert_eq!(context.base_url, "https://flag.example");
    assert!(context.base_url_explicit);
}

#[test]
fn environment_beats_stored_config() {
    let home = tempfile::tempdir().unwrap();
    store_base_url(home.path(), "https://stored.example");
    let environment = FakeEnvironment::with_home(home.path().to_path_buf())
        .with_variable("LITELLM_PROXY_URL", "https://environment.example");
    let context = resolve(&environment, flags(None, None), Some("models")).unwrap();
    assert_eq!(context.base_url, "https://environment.example");
    assert!(context.base_url_explicit);
}

#[test]
fn stored_config_is_used_without_a_flag_or_environment() {
    let home = tempfile::tempdir().unwrap();
    store_base_url(home.path(), "https://stored.example/");
    let environment = FakeEnvironment::with_home(home.path().to_path_buf());
    let context = resolve(&environment, flags(None, None), Some("models")).unwrap();
    assert_eq!(context.base_url, "https://stored.example");
    assert!(context.base_url_explicit);
}

#[test]
fn an_empty_stored_base_url_counts_as_unset() {
    let home = tempfile::tempdir().unwrap();
    store_base_url(home.path(), "");
    let environment = FakeEnvironment::with_home(home.path().to_path_buf());
    let context = resolve(&environment, flags(None, None), Some("models")).unwrap();
    assert_eq!(context.base_url, "http://localhost:4000");
    assert!(!context.base_url_explicit);
}

#[test]
fn trailing_slashes_are_stripped() {
    let context = resolve(
        &FakeEnvironment::default(),
        flags(Some("https://flag.example///"), None),
        Some("models"),
    )
    .unwrap();
    assert_eq!(context.base_url, "https://flag.example");
}

#[test]
fn empty_environment_values_count_as_unset() {
    let environment = FakeEnvironment::default()
        .with_variable("LITELLM_PROXY_URL", "")
        .with_variable("LITELLM_PROXY_API_KEY", "");
    let context = resolve(&environment, flags(None, None), Some("models")).unwrap();
    assert_eq!(context.base_url, "http://localhost:4000");
    assert!(context.api_key.is_none());
    assert!(context.api_key_from_token_file);
}

#[test]
fn explicit_api_keys_are_kept_secret_and_used_as_given() {
    let context = resolve(
        &FakeEnvironment::default(),
        flags(None, Some("flag-key")),
        Some("models"),
    )
    .unwrap();
    assert_eq!(
        context.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("flag-key")
    );
    assert!(!context.api_key_from_token_file);
}

#[test]
fn environment_api_key_is_used_when_no_flag_gives_one() {
    let environment =
        FakeEnvironment::default().with_variable("LITELLM_PROXY_API_KEY", "environment-key");
    let context = resolve(&environment, flags(None, None), Some("models")).unwrap();
    assert_eq!(
        context.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("environment-key")
    );
    assert!(!context.api_key_from_token_file);
}

#[test]
fn an_invalid_stored_base_url_is_a_usage_error() {
    let home = tempfile::tempdir().unwrap();
    store_base_url(home.path(), "not a url");
    let environment = FakeEnvironment::with_home(home.path().to_path_buf());
    let failure = resolve(&environment, flags(None, None), Some("models")).unwrap_err();
    assert!(matches!(failure, Failure::Usage(_)));
}

#[test]
fn an_invalid_config_file_is_ignored() {
    let home = tempfile::tempdir().unwrap();
    write_config(home.path(), "{not json");
    let environment = FakeEnvironment::with_home(home.path().to_path_buf());
    let context = resolve(&environment, flags(None, None), Some("models")).unwrap();
    assert_eq!(context.base_url, "http://localhost:4000");
}

#[test]
fn missing_home_skips_the_stored_token_lookup() {
    let context = resolve(
        &FakeEnvironment::default(),
        flags(None, None),
        Some("models"),
    )
    .unwrap();
    assert!(context.api_key_from_token_file);
    assert!(context.api_key.is_none());
}

const STORED_PROXY: &str = "https://gateway.example";

fn token_record() -> CliTokenRecord {
    CliTokenRecord {
        base_url: String::from(STORED_PROXY),
        key: Some(SecretString::from("sk-secret")),
        user_id: String::from("user-1"),
        user_email: String::from("user@example.com"),
        user_role: String::from("cli"),
        auth_header_name: String::from("Authorization"),
        jwt_token: SecretString::default(),
        timestamp: 1_000.0,
        expires_at: None,
        refresh_token: None,
        client_id: None,
        token_endpoint: None,
        revocation_endpoint: None,
        resource: None,
        team_id: None,
    }
}

#[test]
fn a_stored_login_supplies_the_api_key() {
    let home = tempfile::tempdir().unwrap();
    store_base_url(home.path(), STORED_PROXY);
    let token_path = token_file_path(home.path());
    let files = ManagedTokenFile {
        files: PrivateFiles::default(),
        path: token_path.clone(),
    };
    let mut vault = WorkingVault::default();
    let outcome = {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(save_cli_token(&files, &mut vault, token_record()))
    };
    assert!(matches!(outcome, SecretSave::SecretStored));
    let environment = FakeEnvironment::with_home(home.path().to_path_buf());
    let stderr = Captured::default();
    let context = context::resolve_for_run(&mut Request {
        environment: &environment,
        globals: flags(None, None),
        subcommand: Some("models"),
        vault: &mut vault,
        clock: Arc::new(CliClock),
        token_path: Some(token_path.as_path()),
        stderr: channel(Box::new(stderr)),
    })
    .unwrap();
    assert_eq!(
        context.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-secret")
    );
}
