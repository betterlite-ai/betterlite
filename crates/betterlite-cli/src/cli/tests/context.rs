use std::fs;
use std::path::Path;
use std::sync::Arc;

use secrecy::{ExposeSecret as _, SecretString};

use crate::cli::Failure;
use crate::cli::context::{self, GlobalArgs, Request};
use crate::cli::session::CliClock;
use crate::cli::tests::support::{Captured, FakeEnvironment, FakeVault, channel};
use crate::cli_context::CliContext;

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
    context::resolve_for_run(Request {
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
        context.api_key.as_ref().map(|key| key.expose_secret()),
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
        context.api_key.as_ref().map(|key| key.expose_secret()),
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
