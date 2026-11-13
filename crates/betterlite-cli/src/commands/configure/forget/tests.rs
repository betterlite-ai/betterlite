#![cfg(test)]

use std::path::PathBuf;

use clap::Parser as _;
use clap::error::ErrorKind;
use tempfile::TempDir;

use crate::agent_setup_profiles::{
    Target, receipt_path_for, settings_path_for, setup_profile_path,
};
use crate::commands::configure::args::{UnconfigureArgs, UnconfigureTarget};
use crate::commands::configure::test_support::{
    FakeEnvironment, Harness, MockProxy, TestCli, TestCommand, context, listing, save_profile,
    target_configure,
};

fn profile_path(target: Target, environment: &FakeEnvironment) -> PathBuf {
    let settings = settings_path_for(target, environment).unwrap();
    setup_profile_path(target, &settings, environment).unwrap()
}

#[tokio::test]
async fn forget_flag_deletes_the_saved_profile() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    save_profile(
        Target::Claude,
        &environment,
        "http://proxy.test",
        Some("claude-1"),
    )
    .await;
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let args = UnconfigureArgs {
        forget: true,
        command: Some(UnconfigureTarget::Claude { forget: false }),
    };
    harness.unconfigure(args).await.unwrap();
    assert!(!profile_path(Target::Claude, &harness.environment).exists());
    assert!(
        harness
            .stdout_text()
            .contains("Forgot saved claude setup, including its saved key.")
    );
}

#[tokio::test]
async fn unconfigure_without_forget_retains_the_saved_profile() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    save_profile(
        Target::Claude,
        &environment,
        "http://proxy.test",
        Some("claude-1"),
    )
    .await;
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let args = UnconfigureArgs {
        forget: false,
        command: Some(UnconfigureTarget::Claude { forget: false }),
    };
    harness.unconfigure(args).await.unwrap();
    assert!(profile_path(Target::Claude, &harness.environment).exists());
    assert!(
        harness
            .stdout_text()
            .contains("Saved setup retained. Run `lite configure claude` to apply it again.")
    );
}

#[tokio::test]
async fn parent_forget_reaches_the_subcommand() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    save_profile(
        Target::Claude,
        &environment,
        "http://proxy.test",
        Some("claude-1"),
    )
    .await;
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let args = UnconfigureArgs {
        forget: true,
        command: Some(UnconfigureTarget::Claude { forget: false }),
    };
    harness.unconfigure(args).await.unwrap();
    assert!(!profile_path(Target::Claude, &harness.environment).exists());
}

#[test]
fn forget_flag_is_rejected_by_configure_and_reconfigure() {
    let configure_error = TestCli::try_parse_from(["lite", "configure", "claude", "--forget"])
        .err()
        .unwrap();
    assert_eq!(configure_error.kind(), ErrorKind::UnknownArgument);
    let group_error = TestCli::try_parse_from(["lite", "configure", "--forget"])
        .err()
        .unwrap();
    assert_eq!(group_error.kind(), ErrorKind::UnknownArgument);
    let reconfigure_error = TestCli::try_parse_from(["lite", "reconfigure", "codex", "--forget"])
        .err()
        .unwrap();
    assert_eq!(reconfigure_error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn forget_flag_parses_on_unconfigure_and_its_subcommands() {
    let parsed = TestCli::try_parse_from(["lite", "unconfigure", "--forget", "codex"]).unwrap();
    let TestCommand::Unconfigure(args) = parsed.command else {
        panic!("expected the unconfigure command");
    };
    assert!(args.forget);
    let Some(UnconfigureTarget::Codex {
        forget: codex_forget,
    }) = args.command
    else {
        panic!("expected the codex subcommand");
    };
    assert!(!codex_forget);
    let subcommand =
        TestCli::try_parse_from(["lite", "unconfigure", "claude", "--forget"]).unwrap();
    let TestCommand::Unconfigure(subcommand_args) = subcommand.command else {
        panic!("expected the unconfigure command");
    };
    assert!(!subcommand_args.forget);
    let Some(UnconfigureTarget::Claude {
        forget: claude_forget,
    }) = subcommand_args.command
    else {
        panic!("expected the claude subcommand");
    };
    assert!(claude_forget);
}

#[tokio::test]
async fn forget_after_configure_removes_the_profile_and_the_receipt() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    harness
        .configure(target_configure(
            Target::Claude,
            &proxy.base_url,
            "sk-test",
            "claude-1",
        ))
        .await
        .unwrap();
    let settings_path = settings_path_for(Target::Claude, &harness.environment).unwrap();
    let state_path =
        receipt_path_for(Target::Claude, &settings_path, &harness.environment).unwrap();
    assert!(profile_path(Target::Claude, &harness.environment).exists());
    let args = UnconfigureArgs {
        forget: true,
        command: None,
    };
    harness.unconfigure(args).await.unwrap();
    assert!(!profile_path(Target::Claude, &harness.environment).exists());
    assert!(!state_path.exists());
    assert!(
        harness
            .stdout_text()
            .contains("Forgot saved claude setup, including its saved key.")
    );
}

#[tokio::test]
async fn parent_forget_reaches_the_codex_subcommand() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    save_profile(
        Target::Codex,
        &environment,
        "http://proxy.test",
        Some("codex-1"),
    )
    .await;
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let args = UnconfigureArgs {
        forget: true,
        command: Some(UnconfigureTarget::Codex { forget: false }),
    };
    harness.unconfigure(args).await.unwrap();
    assert!(!profile_path(Target::Codex, &harness.environment).exists());
}
