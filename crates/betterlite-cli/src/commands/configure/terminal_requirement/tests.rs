#![cfg(test)]

use tempfile::TempDir;

use crate::agent_setup_profiles::Target;
use crate::commands::configure::args::{ConnectionFlags, ReconfigureArgs, ReconfigureTarget};
use crate::commands::configure::error::ConfigureCommandError;
use crate::commands::configure::test_support::{
    FakeEnvironment, Harness, MockProxy, context, group_configure, group_connection, listing,
    save_profile, target_configure,
};

const CONFIGURE_MESSAGE: &str = "`lite configure` asks questions, so it needs a terminal. Non-interactively, run `lite configure claude --api-key <key> --model <model>` or `lite configure codex --api-key <key> --model <model>`";

#[tokio::test]
async fn require_terminal_stops_a_first_time_configure() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let mut harness = Harness::new(environment, context("http://proxy.test", Some("sk-test")));
    let error = harness.configure(group_configure()).await.unwrap_err();
    assert_eq!(error.to_string(), CONFIGURE_MESSAGE);
    assert!(matches!(
        &error,
        ConfigureCommandError::Terminal {
            command: "configure"
        }
    ));
    assert!(harness.prompter.calls.is_empty());
}

#[tokio::test]
async fn require_terminal_stops_a_reconfigure_wizard() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let mut harness = Harness::new(environment, context("http://proxy.test", Some("sk-test")));
    let args = ReconfigureArgs {
        connection: group_connection(None, None),
        command: None,
    };
    let error = harness.reconfigure(args).await.unwrap_err();
    assert!(matches!(
        &error,
        ConfigureCommandError::Terminal {
            command: "reconfigure"
        }
    ));
    assert!(harness.prompter.calls.is_empty());
}

#[tokio::test]
async fn configure_with_flags_runs_without_a_terminal() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    harness
        .configure(target_configure(
            Target::Claude,
            &proxy.base_url,
            "sk-new",
            "claude-1",
        ))
        .await
        .unwrap();
    assert!(harness.prompter.calls.is_empty());
}

#[tokio::test]
async fn reconfigure_with_a_model_runs_without_a_terminal() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    let args = ReconfigureArgs {
        connection: group_connection(None, None),
        command: Some(ReconfigureTarget::Claude {
            connection: ConnectionFlags::from(group_connection(
                Some("sk-new"),
                Some(&proxy.base_url),
            )),
            model: Some("claude-1".to_owned()),
            default_model: false,
        }),
    };
    harness.reconfigure(args).await.unwrap();
    assert!(harness.prompter.calls.is_empty());
}

#[tokio::test]
async fn saved_setup_reapplies_without_a_terminal() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    save_profile(
        Target::Claude,
        &environment,
        &proxy.base_url,
        Some("claude-1"),
    )
    .await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    harness.configure(group_configure()).await.unwrap();
    assert!(harness.prompter.calls.is_empty());
}
