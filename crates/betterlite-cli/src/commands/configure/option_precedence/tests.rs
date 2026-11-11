#![cfg(test)]

use clap::Parser as _;
use secrecy::ExposeSecret as _;
use tempfile::TempDir;

use crate::commands::configure::args::{
    ConfigureArgs, ConfigureTarget, ConnectionFlags, ReconfigureArgs, ReconfigureTarget,
};
use crate::commands::configure::connection::connection_settings;
use crate::commands::configure::error::ConfigureCommandError;
use crate::commands::configure::test_support::{
    FakeEnvironment, Harness, MockProxy, TestCli, TestCommand, claude_env, claude_settings,
    codex_config, context, group_connection, implicit_context, listing, token_context,
};

#[test]
fn flag_precedence_subcommand_beats_group_and_root() {
    let root = context("http://root.test", Some("sk-root"));
    let group = group_connection(Some("sk-group"), Some("http://group.test"));
    let flags = ConnectionFlags::from(group_connection(Some("sk-sub"), Some("http://sub.test")));
    let merged = connection_settings(
        &root,
        Some(&group),
        flags.api_key.as_ref(),
        flags.gateway_url.as_deref(),
    )
    .unwrap();
    assert_eq!(merged.base_url, "http://sub.test");
    assert!(merged.base_url_explicit);
    assert_eq!(merged.api_key.unwrap().expose_secret(), "sk-sub");
    assert!(!merged.api_key_from_token_file);
}

#[test]
fn flag_precedence_group_beats_root() {
    let root = context("http://root.test", Some("sk-root"));
    let group = group_connection(Some("sk-group"), Some("http://group.test"));
    let flags = ConnectionFlags::from(group_connection(None, None));
    let merged = connection_settings(
        &root,
        Some(&group),
        flags.api_key.as_ref(),
        flags.gateway_url.as_deref(),
    )
    .unwrap();
    assert_eq!(merged.base_url, "http://group.test");
    assert_eq!(merged.api_key.unwrap().expose_secret(), "sk-group");
}

#[test]
fn flag_precedence_uses_the_root_without_any_flag() {
    let root = context("http://root.test", Some("sk-root"));
    let group = group_connection(None, None);
    let flags = ConnectionFlags::from(group_connection(None, None));
    let merged = connection_settings(
        &root,
        Some(&group),
        flags.api_key.as_ref(),
        flags.gateway_url.as_deref(),
    )
    .unwrap();
    assert_eq!(merged.base_url, "http://root.test");
    assert_eq!(merged.api_key.unwrap().expose_secret(), "sk-root");
}

#[test]
fn option_merge_keeps_the_token_file_flag_for_a_root_key() {
    let root = token_context("http://root.test", "sk-login");
    let merged = connection_settings(&root, None, None, None).unwrap();
    assert!(merged.api_key_from_token_file);
    assert_eq!(merged.api_key.unwrap().expose_secret(), "sk-login");
}

#[test]
fn option_merge_clears_the_token_file_flag_for_a_group_key() {
    let root = token_context("http://root.test", "sk-login");
    let group = group_connection(Some("sk-group"), None);
    let merged = connection_settings(&root, Some(&group), None, None).unwrap();
    assert!(!merged.api_key_from_token_file);
    assert_eq!(merged.api_key.unwrap().expose_secret(), "sk-group");
}

#[test]
fn option_merge_strips_a_trailing_v1() {
    let root = context("http://root.test", None);
    let group = group_connection(None, Some("http://group.test/v1"));
    let merged = connection_settings(&root, Some(&group), None, None).unwrap();
    assert_eq!(merged.base_url, "http://group.test");
}

#[test]
fn option_merge_marks_a_flagged_url_explicit() {
    let root = implicit_context("http://stored.test", None);
    let merged = connection_settings(&root, None, None, Some("http://flag.test")).unwrap();
    assert!(merged.base_url_explicit);
    assert_eq!(merged.base_url, "http://flag.test");
}

#[test]
fn option_merge_rejects_an_invalid_gateway_url() {
    let root = context("http://root.test", None);
    let error = connection_settings(&root, None, None, Some("ftp://proxy.test")).unwrap_err();
    assert!(matches!(&error, ConfigureCommandError::Usage(_)));
    assert_eq!(
        error.to_string(),
        "base_url must be a full http:// or https:// URL including a host"
    );
}

#[test]
fn flag_precedence_parses_both_gateway_url_levels() {
    let parsed = TestCli::try_parse_from([
        "lite",
        "configure",
        "--base-url",
        "http://group.test",
        "claude",
        "--gateway-url",
        "http://sub.test",
    ])
    .unwrap();
    let TestCommand::Configure(args) = parsed.command else {
        panic!("expected the configure command");
    };
    assert_eq!(
        args.connection.gateway_url.as_deref(),
        Some("http://group.test")
    );
    let Some(ConfigureTarget::Claude { connection, .. }) = args.command else {
        panic!("expected the claude subcommand");
    };
    assert_eq!(connection.gateway_url.as_deref(), Some("http://sub.test"));
}

#[tokio::test]
async fn option_merge_group_flags_reach_a_configure_subcommand() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let mut harness = Harness::new(environment, context("http://login.test", Some("sk-login")));
    let args = ConfigureArgs {
        connection: group_connection(Some("sk-group"), Some(&proxy.base_url)),
        command: Some(ConfigureTarget::Claude {
            connection: ConnectionFlags::from(group_connection(None, None)),
            model: Some("claude-1".to_owned()),
            default_model: false,
        }),
    };
    harness.configure(args).await.unwrap();
    assert!(harness.prompter.calls.is_empty());
    assert_eq!(proxy.requests(), vec!["/v1/models".to_owned()]);
    let settings = claude_settings(&harness.environment);
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_BASE_URL").as_deref(),
        Some(proxy.base_url.as_str())
    );
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_AUTH_TOKEN").as_deref(),
        Some("sk-group")
    );
}

#[tokio::test]
async fn option_merge_group_flags_reach_a_reconfigure_subcommand() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("codex-1", None)])]).await;
    let mut harness = Harness::new(environment, context("http://login.test", Some("sk-login")));
    let args = ReconfigureArgs {
        connection: group_connection(Some("sk-group"), Some(&proxy.base_url)),
        command: Some(ReconfigureTarget::Codex {
            connection: ConnectionFlags::from(group_connection(None, None)),
            model: Some("codex-1".to_owned()),
        }),
    };
    harness.reconfigure(args).await.unwrap();
    assert!(harness.prompter.calls.is_empty());
    assert_eq!(proxy.requests(), vec!["/v1/models".to_owned()]);
    let config = codex_config(&harness.environment);
    assert!(config.contains(proxy.base_url.as_str()));
    assert!(config.contains("sk-group"));
}
