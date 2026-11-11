#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;

use betterlite_config::sections::settings::JsonValue;
use tempfile::TempDir;

use crate::agent_setup_profiles::{Target, receipt_path_for, settings_path_for};
use crate::commands::configure::args::{UnconfigureArgs, UnconfigureTarget};
use crate::commands::configure::test_support::{
    FakeEnvironment, Harness, MockProxy, claude_env, claude_settings, context, listing,
    target_configure, unconfigure_claude, write_claude_settings,
};

#[tokio::test]
async fn unconfigure_no_receipt_reports_nothing_to_undo() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    harness.unconfigure(unconfigure_claude()).await.unwrap();
    let settings_path = settings_path_for(Target::Claude, &harness.environment).unwrap();
    let state_path =
        receipt_path_for(Target::Claude, &settings_path, &harness.environment).unwrap();
    assert_eq!(
        harness.stdout_text(),
        format!(
            "No claude undo receipt at {}; nothing to undo. Agent settings were not changed.\n",
            state_path.display()
        )
    );
    assert!(harness.stderr_text().is_empty());
}

#[tokio::test]
async fn unconfigure_no_receipt_points_at_the_settings_to_clean_up() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    fs::write(&settings_path, "{}").unwrap();
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    harness.unconfigure(unconfigure_claude()).await.unwrap();
    assert_eq!(
        harness.stderr_text(),
        format!(
            "Cannot confirm disconnection. Check {} and remove any remaining gateway connection and key manually.\n",
            settings_path.display()
        )
    );
}

#[tokio::test]
async fn unconfigure_group_reports_each_target_without_a_receipt() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let claude_settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    let claude_state =
        receipt_path_for(Target::Claude, &claude_settings_path, &environment).unwrap();
    let codex_settings_path = settings_path_for(Target::Codex, &environment).unwrap();
    let codex_state = receipt_path_for(Target::Codex, &codex_settings_path, &environment).unwrap();
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let args = UnconfigureArgs {
        forget: false,
        command: None,
    };
    harness.unconfigure(args).await.unwrap();
    assert_eq!(
        harness.stdout_text(),
        format!(
            "No claude undo receipt at {}; nothing to undo. Agent settings were not changed.\nNo codex undo receipt at {}; nothing to undo. Agent settings were not changed.\n",
            claude_state.display(),
            codex_state.display()
        )
    );
    assert!(harness.stderr_text().is_empty());
}

#[tokio::test]
async fn unconfigure_reports_settings_modified_since_configure() {
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
    let mut settings = claude_settings(&harness.environment);
    let env = settings.get_mut("env").unwrap();
    env.as_object_mut().unwrap().insert(
        "ANTHROPIC_MODEL".to_owned(),
        JsonValue::String("changed".to_owned()),
    );
    write_claude_settings(&harness.environment, &settings);
    let settings_path = settings_path_for(Target::Claude, &harness.environment).unwrap();
    let state_path =
        receipt_path_for(Target::Claude, &settings_path, &harness.environment).unwrap();
    harness.unconfigure(unconfigure_claude()).await.unwrap();
    let stdout = harness.stdout_text();
    assert!(stdout.contains(&format!("Restored in {}: ", settings_path.display())));
    assert!(stdout.contains("Left as you changed them since: env.ANTHROPIC_MODEL."));
    assert!(!state_path.exists());
    let restored = claude_settings(&harness.environment);
    assert_eq!(
        claude_env(&restored, "ANTHROPIC_MODEL").as_deref(),
        Some("changed")
    );
    assert_eq!(claude_env(&restored, "ANTHROPIC_BASE_URL"), None);
    assert_eq!(claude_env(&restored, "ANTHROPIC_AUTH_TOKEN"), None);
}

#[tokio::test]
async fn unconfigure_reports_when_nothing_is_left_to_restore() {
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
    let mut settings = BTreeMap::new();
    settings.insert("theme".to_owned(), JsonValue::String("dark".to_owned()));
    write_claude_settings(&harness.environment, &settings);
    let settings_path = settings_path_for(Target::Claude, &harness.environment).unwrap();
    let state_path =
        receipt_path_for(Target::Claude, &settings_path, &harness.environment).unwrap();
    harness.unconfigure(unconfigure_claude()).await.unwrap();
    let stdout = harness.stdout_text();
    assert!(stdout.contains(&format!(
        "Nothing in {} was still ours to restore.",
        settings_path.display()
    )));
    assert!(!stdout.contains("Left as you changed them since"));
    assert!(!state_path.exists());
    let kept = claude_settings(&harness.environment);
    assert_eq!(
        kept.get("theme"),
        Some(&JsonValue::String("dark".to_owned()))
    );
}

#[tokio::test]
async fn unconfigure_codex_removes_a_settings_file_it_created() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("codex-1", None)])]).await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    harness
        .configure(target_configure(
            Target::Codex,
            &proxy.base_url,
            "sk-test",
            "codex-1",
        ))
        .await
        .unwrap();
    let settings_path = settings_path_for(Target::Codex, &harness.environment).unwrap();
    let state_path = receipt_path_for(Target::Codex, &settings_path, &harness.environment).unwrap();
    let args = UnconfigureArgs {
        forget: false,
        command: Some(UnconfigureTarget::Codex { forget: false }),
    };
    harness.unconfigure(args).await.unwrap();
    let stdout = harness.stdout_text();
    assert!(stdout.contains(&format!(
        "Removed {}; it held only settings created by `lite configure codex`.",
        settings_path.display()
    )));
    assert!(!settings_path.exists());
    assert!(!state_path.exists());
}

#[tokio::test]
async fn unconfigure_codex_reports_settings_modified_since_configure() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("codex-1", None)])]).await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    harness
        .configure(target_configure(
            Target::Codex,
            &proxy.base_url,
            "sk-test",
            "codex-1",
        ))
        .await
        .unwrap();
    let settings_path = settings_path_for(Target::Codex, &harness.environment).unwrap();
    let text = fs::read_to_string(&settings_path).unwrap();
    fs::write(&settings_path, text.replace("codex-1", "other")).unwrap();
    let args = UnconfigureArgs {
        forget: false,
        command: Some(UnconfigureTarget::Codex { forget: false }),
    };
    harness.unconfigure(args).await.unwrap();
    let stdout = harness.stdout_text();
    assert!(stdout.contains(&format!("Restored in {}: ", settings_path.display())));
    assert!(stdout.contains("Left as you changed them since: model."));
    assert!(
        fs::read_to_string(&settings_path)
            .unwrap()
            .contains("other")
    );
}

#[tokio::test]
async fn unconfigure_reports_an_unreadable_receipt() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    fs::write(&settings_path, "{}").unwrap();
    let state_path = receipt_path_for(Target::Claude, &settings_path, &environment).unwrap();
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    fs::write(&state_path, "not json").unwrap();
    let mut harness = Harness::new(environment, context("http://proxy.test", None));
    let error = harness.unconfigure(unconfigure_claude()).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("is not a readable `lite configure claude` receipt")
    );
}

#[tokio::test]
async fn unconfigure_codex_reports_when_nothing_is_left_to_restore() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("codex-1", None)])]).await;
    let mut harness = Harness::new(environment, context(&proxy.base_url, None));
    harness
        .configure(target_configure(
            Target::Codex,
            &proxy.base_url,
            "sk-test",
            "codex-1",
        ))
        .await
        .unwrap();
    let settings_path = settings_path_for(Target::Codex, &harness.environment).unwrap();
    fs::write(&settings_path, "model = \"mine\"\n").unwrap();
    let args = UnconfigureArgs {
        forget: false,
        command: Some(UnconfigureTarget::Codex { forget: false }),
    };
    harness.unconfigure(args).await.unwrap();
    let stdout = harness.stdout_text();
    assert!(stdout.contains(&format!(
        "Nothing in {} was still ours to restore.",
        settings_path.display()
    )));
    assert!(!stdout.contains("Restored in"));
    assert!(fs::read_to_string(&settings_path).unwrap().contains("mine"));
}
