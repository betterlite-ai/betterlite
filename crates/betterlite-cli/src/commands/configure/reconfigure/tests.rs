#![cfg(test)]

use tempfile::TempDir;

use crate::agent_setup::connection::KEEP_CURRENT_KEY;
use crate::agent_setup::pickers::{CLAUDE_MODEL_MESSAGE, KEEP_CLAUDE_DEFAULT, TARGET_EDIT_MESSAGE};
use crate::agent_setup_profiles::locks::lock_path_for;
use crate::agent_setup_profiles::{Target, settings_path_for};
use crate::commands::configure::args::{ConnectionFlags, ReconfigureArgs, ReconfigureTarget};
use crate::commands::configure::error::ConfigureCommandError;
use crate::commands::configure::test_support::{
    Call, FakeEnvironment, Harness, MockProxy, Reply, claude_env, claude_settings, context,
    group_connection, implicit_context, listing, save_profile,
};

const TERMINAL_MESSAGE: &str = "`lite reconfigure` asks questions, so it needs a terminal. Non-interactively, run `lite reconfigure claude --api-key <key> --model <model>` or `lite reconfigure codex --api-key <key> --model <model>`";

fn edit_calls(gateway_url: &str) -> Vec<Call> {
    vec![
        Call::Url {
            message: "Claude gateway URL".to_owned(),
            default: gateway_url.to_owned(),
        },
        Call::Secret {
            message: KEEP_CURRENT_KEY.to_owned(),
        },
        Call::Model {
            message: CLAUDE_MODEL_MESSAGE.to_owned(),
            choices: vec![KEEP_CLAUDE_DEFAULT.to_owned(), "claude-1".to_owned()],
            selected: 1,
        },
    ]
}

fn claude_edit(
    api_key: Option<&str>,
    gateway_url: Option<&str>,
    model: Option<&str>,
    default_model: bool,
) -> ReconfigureArgs {
    ReconfigureArgs {
        connection: group_connection(None, None),
        command: Some(ReconfigureTarget::Claude {
            connection: ConnectionFlags::from(group_connection(api_key, gateway_url)),
            model: model.map(str::to_owned),
            default_model,
        }),
    }
}

#[tokio::test]
async fn reconfigure_claude_without_model_opens_the_edit_dialog() {
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
    let mut harness = Harness::new(environment, implicit_context(&proxy.base_url, None));
    harness.stdin_is_terminal = true;
    harness.reply(Reply::Url(proxy.base_url.clone()));
    harness.reply(Reply::Model("claude-1".to_owned()));
    harness
        .reconfigure(claude_edit(None, None, None, false))
        .await
        .unwrap();
    assert_eq!(harness.prompter.calls, edit_calls(&proxy.base_url));
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

#[tokio::test]
async fn reconfigure_claude_with_model_applies_without_prompting() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    harness
        .reconfigure(claude_edit(
            Some("sk-new"),
            Some(&proxy.base_url),
            Some("claude-1"),
            false,
        ))
        .await
        .unwrap();
    assert!(harness.prompter.calls.is_empty());
    let settings = claude_settings(&harness.environment);
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_AUTH_TOKEN").as_deref(),
        Some("sk-new")
    );
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_MODEL").as_deref(),
        Some("claude-1")
    );
}

#[tokio::test]
async fn reconfigure_claude_with_default_model_unpins_without_prompting() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    harness
        .reconfigure(claude_edit(
            Some("sk-new"),
            Some(&proxy.base_url),
            None,
            true,
        ))
        .await
        .unwrap();
    assert!(harness.prompter.calls.is_empty());
    let settings = claude_settings(&harness.environment);
    assert_eq!(claude_env(&settings, "ANTHROPIC_MODEL"), None);
    assert_eq!(settings.get("model"), None);
}

#[tokio::test]
async fn reconfigure_codex_without_model_requires_a_terminal() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let args = ReconfigureArgs {
        connection: group_connection(None, None),
        command: Some(ReconfigureTarget::Codex {
            connection: ConnectionFlags::from(group_connection(
                Some("sk-new"),
                Some("http://proxy.test"),
            )),
            model: None,
        }),
    };
    let error = harness.reconfigure(args).await.unwrap_err();
    assert_eq!(error.to_string(), TERMINAL_MESSAGE);
    assert!(matches!(
        &error,
        ConfigureCommandError::Terminal {
            command: "reconfigure"
        }
    ));
    assert!(harness.prompter.calls.is_empty());
    assert!(
        !lock_path_for(Target::Codex, &harness.environment)
            .unwrap()
            .exists()
    );
    assert!(
        !settings_path_for(Target::Codex, &harness.environment)
            .unwrap()
            .exists()
    );
}

#[tokio::test]
async fn reconfigure_group_with_saved_setups_edits_instead_of_reapplying() {
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
    let mut harness = Harness::new(environment, implicit_context(&proxy.base_url, None));
    harness.stdin_is_terminal = true;
    harness.reply(Reply::Targets(vec![Target::Claude]));
    harness.reply(Reply::Url(proxy.base_url.clone()));
    harness.reply(Reply::Model("claude-1".to_owned()));
    let args = ReconfigureArgs {
        connection: group_connection(None, None),
        command: None,
    };
    harness.reconfigure(args).await.unwrap();
    assert_eq!(
        harness.prompter.calls[0],
        Call::Targets {
            message: TARGET_EDIT_MESSAGE.to_owned(),
            defaults: vec![Target::Claude],
            on_empty: "Pick at least one.".to_owned(),
        }
    );
    assert!(harness.prompter.calls.len() > 1);
    let settings = claude_settings(&harness.environment);
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_AUTH_TOKEN").as_deref(),
        Some("sk-saved")
    );
}

#[tokio::test]
async fn reconfigure_rejects_model_with_default_model() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let error = harness
        .reconfigure(claude_edit(
            Some("sk-new"),
            Some("http://proxy.test"),
            Some("claude-1"),
            true,
        ))
        .await
        .unwrap_err();
    assert!(matches!(&error, ConfigureCommandError::Usage(_)));
    assert!(
        error
            .to_string()
            .contains("--model and --default-model cannot be used together")
    );
}

#[tokio::test]
async fn reconfigure_without_a_subcommand_still_needs_a_terminal_for_its_group() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let args = ReconfigureArgs {
        connection: group_connection(None, None),
        command: None,
    };
    let error = harness.reconfigure(args).await.unwrap_err();
    assert_eq!(error.to_string(), TERMINAL_MESSAGE);
    assert!(
        !settings_path_for(Target::Claude, &harness.environment)
            .unwrap()
            .exists()
    );
}
