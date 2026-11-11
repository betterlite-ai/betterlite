#![cfg(test)]

use std::fs;

use tempfile::TempDir;

use crate::agent_setup::pickers::{
    CLAUDE_MODEL_MESSAGE, EMPTY_TARGETS_MESSAGE, KEEP_CLAUDE_DEFAULT, TARGET_ROUTE_MESSAGE,
};
use crate::agent_setup_profiles::locks::lock_path_for;
use crate::agent_setup_profiles::{Target, settings_path_for, setup_profile_path};
use crate::commands::configure::args::{ConfigureArgs, ConfigureTarget, ConnectionFlags};
use crate::commands::configure::test_support::{
    Call, FakeEnvironment, Harness, MockProxy, Reply, claude_env, claude_settings, codex_config,
    context, group_configure, group_connection, implicit_context, listing, save_profile,
};

#[tokio::test]
async fn group_invocation_applies_both_saved_setups() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![
        listing(&[("claude-1", None)]),
        listing(&[("codex-1", None)]),
    ])
    .await;
    let url = &proxy.base_url;
    save_profile(Target::Claude, &environment, url, Some("claude-1")).await;
    save_profile(Target::Codex, &environment, url, Some("codex-1")).await;
    let mut harness = Harness::new(environment, context(url, None));
    harness.configure(group_configure()).await.unwrap();
    assert!(harness.prompter.calls.is_empty());
    let stdout = harness.stdout_text();
    assert!(stdout.contains("Configured Claude Code"));
    assert!(stdout.contains("Configured Codex"));
    assert_eq!(proxy.requests(), vec!["/v1/models".to_owned(); 2]);
    let settings = claude_settings(&harness.environment);
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_BASE_URL").as_deref(),
        Some(proxy.base_url.as_str())
    );
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_AUTH_TOKEN").as_deref(),
        Some("sk-saved")
    );
    assert!(codex_config(&harness.environment).contains("codex-1"));
    assert!(
        lock_path_for(Target::Claude, &harness.environment)
            .unwrap()
            .exists()
    );
    assert!(
        lock_path_for(Target::Codex, &harness.environment)
            .unwrap()
            .exists()
    );
}

#[tokio::test]
async fn group_invocation_applies_only_the_saved_target() {
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
    assert_eq!(proxy.requests().len(), 1);
    assert!(
        !settings_path_for(Target::Codex, &harness.environment)
            .unwrap()
            .exists()
    );
}

#[tokio::test]
async fn group_invocation_reapplies_the_saved_setup() {
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
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    fs::write(&settings_path, "{}").unwrap();
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    harness.configure(group_configure()).await.unwrap();
    let settings = claude_settings(&harness.environment);
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_BASE_URL").as_deref(),
        Some(proxy.base_url.as_str())
    );
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_AUTH_TOKEN").as_deref(),
        Some("sk-saved")
    );
}

#[tokio::test]
async fn group_invocation_without_saved_setup_opens_the_wizard() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, Some("sk-test")));
    harness.stdin_is_terminal = true;
    harness.reply(Reply::Targets(vec![Target::Claude]));
    harness.reply(Reply::Model("claude-1".to_owned()));
    harness.configure(group_configure()).await.unwrap();
    assert_eq!(
        harness.prompter.calls,
        vec![
            Call::Targets {
                message: TARGET_ROUTE_MESSAGE.to_owned(),
                defaults: vec![Target::Claude, Target::Codex],
                on_empty: EMPTY_TARGETS_MESSAGE.to_owned(),
            },
            Call::Model {
                message: CLAUDE_MODEL_MESSAGE.to_owned(),
                choices: vec![KEEP_CLAUDE_DEFAULT.to_owned(), "claude-1".to_owned()],
                selected: 0,
            },
        ]
    );
    let settings = claude_settings(&harness.environment);
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_AUTH_TOKEN").as_deref(),
        Some("sk-test")
    );
}

#[tokio::test]
async fn group_invocation_prompts_for_gateway_when_not_explicit() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let mut harness = Harness::new(
        environment,
        implicit_context("http://stored.test", Some("sk-test")),
    );
    harness.stdin_is_terminal = true;
    harness.reply(Reply::Targets(vec![Target::Claude]));
    harness.reply(Reply::Url(proxy.base_url.clone()));
    harness.reply(Reply::Model("claude-1".to_owned()));
    harness.configure(group_configure()).await.unwrap();
    assert_eq!(
        harness.prompter.calls[1],
        Call::Url {
            message: "Gateway URL".to_owned(),
            default: "http://stored.test".to_owned(),
        }
    );
    assert_eq!(proxy.requests(), vec!["/v1/models".to_owned()]);
    let settings = claude_settings(&harness.environment);
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_BASE_URL").as_deref(),
        Some(proxy.base_url.as_str())
    );
}

#[tokio::test]
async fn subcommand_does_not_apply_saved_setups() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("codex-1", None)])]).await;
    let url = &proxy.base_url;
    save_profile(Target::Claude, &environment, url, Some("claude-1")).await;
    let mut harness = Harness::new(environment, context(url, None));
    let args = ConfigureArgs {
        connection: group_connection(None, None),
        command: Some(ConfigureTarget::Codex {
            connection: ConnectionFlags::from(group_connection(Some("sk-new"), Some(url))),
            model: Some("codex-1".to_owned()),
        }),
    };
    harness.configure(args).await.unwrap();
    assert!(
        lock_path_for(Target::Codex, &harness.environment)
            .unwrap()
            .exists()
    );
    assert!(
        !lock_path_for(Target::Claude, &harness.environment)
            .unwrap()
            .exists()
    );
    assert!(
        !settings_path_for(Target::Claude, &harness.environment)
            .unwrap()
            .exists()
    );
    assert!(
        settings_path_for(Target::Codex, &harness.environment)
            .unwrap()
            .exists()
    );
}

#[tokio::test]
async fn group_invocation_reports_an_invalid_saved_setup() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    save_profile(
        Target::Claude,
        &environment,
        "http://proxy.test",
        Some("claude-1"),
    )
    .await;
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    let profile = setup_profile_path(Target::Claude, &settings_path, &environment).unwrap();
    fs::write(&profile, "{}").unwrap();
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let error = harness.configure(group_configure()).await.unwrap_err();
    assert!(error.to_string().contains("Saved claude setup"));
    assert!(error.to_string().contains("is invalid or unsupported"));
}

#[tokio::test]
async fn configure_subcommand_reapplies_the_saved_profile_without_a_terminal() {
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
    let args = ConfigureArgs {
        connection: group_connection(None, None),
        command: Some(ConfigureTarget::Claude {
            connection: ConnectionFlags::from(group_connection(None, None)),
            model: None,
            default_model: false,
        }),
    };
    harness.configure(args).await.unwrap();
    assert!(harness.prompter.calls.is_empty());
    let settings = claude_settings(&harness.environment);
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_AUTH_TOKEN").as_deref(),
        Some("sk-saved")
    );
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_MODEL").as_deref(),
        Some("claude-1")
    );
}
