use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::cli::context::{self, GlobalArgs, Request};
use crate::cli::session::CliClock;
use crate::cli::streams::Channel;
use crate::cli::tests::support::{Captured, FakeEnvironment, FakeVault};

const SETUP_COMMANDS: [&str; 3] = ["configure", "reconfigure", "unconfigure"];

fn store_base_url(home: &Path, base_url: &str) {
    let directory = home.join(".litellm");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("config.json"),
        format!("{{\"base_url\":\"{base_url}\"}}"),
    )
    .unwrap();
}

fn resolve(
    environment: &FakeEnvironment,
    base_url: Option<&str>,
    subcommand: Option<&str>,
) -> crate::cli_context::CliContext {
    let mut vault = FakeVault;
    let stderr = Captured::default();
    context::resolve_for_run(Request {
        environment,
        globals: GlobalArgs {
            base_url: base_url.map(str::to_owned),
            api_key: None,
        },
        subcommand,
        vault: &mut vault,
        clock: Arc::new(CliClock),
        token_path: None,
        stderr: Channel::new(Box::new(stderr)),
    })
    .unwrap()
}

#[test]
fn flag_marks_the_base_url_explicit() {
    let context = resolve(
        &FakeEnvironment::default(),
        Some("https://flag.example"),
        Some("models"),
    );
    assert!(context.base_url_explicit);
}

#[test]
fn environment_only_leaves_the_base_url_implicit() {
    let environment = FakeEnvironment::default()
        .with_variable("LITELLM_PROXY_URL", "https://environment.example");
    let context = resolve(&environment, None, Some("models"));
    assert!(!context.base_url_explicit);
}

#[test]
fn stored_base_url_is_explicit_for_a_normal_command() {
    let home = tempfile::tempdir().unwrap();
    store_base_url(home.path(), "https://stored.example");
    let environment = FakeEnvironment::with_home(home.path().to_path_buf());
    let context = resolve(&environment, None, Some("models"));
    assert!(context.base_url_explicit);
}

#[test]
fn stored_base_url_is_implicit_inside_setup_commands() {
    let home = tempfile::tempdir().unwrap();
    store_base_url(home.path(), "https://stored.example");
    let environment = FakeEnvironment::with_home(home.path().to_path_buf());
    for command in SETUP_COMMANDS {
        let context = resolve(&environment, None, Some(command));
        assert!(
            !context.base_url_explicit,
            "{command} must not treat the stored URL as explicit"
        );
    }
}

#[test]
fn flag_marks_the_base_url_explicit_inside_setup_commands() {
    let context = resolve(
        &FakeEnvironment::default(),
        Some("https://flag.example"),
        Some("configure"),
    );
    assert!(context.base_url_explicit);
}

#[test]
fn normal_commands_read_the_token_file_when_no_key_is_given() {
    let context = resolve(&FakeEnvironment::default(), None, Some("models"));
    assert!(context.api_key_from_token_file);
}

#[test]
fn setup_commands_never_read_the_token_file() {
    for command in SETUP_COMMANDS {
        let context = resolve(&FakeEnvironment::default(), None, Some(command));
        assert!(
            !context.api_key_from_token_file,
            "{command} must not take its key from the token file"
        );
    }
}

#[test]
fn the_root_invocation_reads_the_token_file() {
    let context = resolve(&FakeEnvironment::default(), None, None);
    assert!(context.api_key_from_token_file);
}
