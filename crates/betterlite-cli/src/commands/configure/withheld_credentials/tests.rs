#![cfg(test)]

use std::fs;

use betterlite_config::sections::settings::JsonValue;
use tempfile::TempDir;

use crate::agent_setup_profiles::{Target, receipt_path_for, settings_path_for};
use crate::commands::configure::test_support::{
    FakeEnvironment, Harness, MockProxy, claude_env, claude_settings, context, listing,
    target_configure, unconfigure_claude, write_claude_settings,
};

fn set_base_url(environment: &FakeEnvironment, url: &str) {
    let mut settings = claude_settings(environment);
    let env = settings.get_mut("env").unwrap();
    env.as_object_mut().unwrap().insert(
        "ANTHROPIC_BASE_URL".to_owned(),
        JsonValue::String(url.to_owned()),
    );
    write_claude_settings(environment, &settings);
}

#[tokio::test]
async fn withheld_credentials_report_their_captured_endpoint() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    fs::write(
        &settings_path,
        format!(
            r#"{{"env":{{"ANTHROPIC_BASE_URL":"{}","ANTHROPIC_AUTH_TOKEN":"sk-old"}}}}"#,
            proxy.base_url
        ),
    )
    .unwrap();
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
    set_base_url(&harness.environment, "http://elsewhere.test");
    let state_path =
        receipt_path_for(Target::Claude, &settings_path, &harness.environment).unwrap();
    harness.unconfigure(unconfigure_claude()).await.unwrap();
    let expected = format!(
        "Left removed, since the file now points at a different server than they were issued for: env.ANTHROPIC_AUTH_TOKEN (captured with {}). They stay in {}: point env.ANTHROPIC_BASE_URL back and run `lite unconfigure claude` again to put them back, or delete that file to drop them.\n",
        proxy.base_url,
        state_path.display()
    );
    assert!(harness.stdout_text().contains(&expected));
    assert!(state_path.exists());
    let settings = claude_settings(&harness.environment);
    assert_eq!(claude_env(&settings, "ANTHROPIC_AUTH_TOKEN"), None);
    assert_eq!(
        claude_env(&settings, "ANTHROPIC_BASE_URL").as_deref(),
        Some("http://elsewhere.test")
    );
}

#[tokio::test]
async fn withheld_credentials_name_the_default_endpoint_when_none_was_captured() {
    let home = TempDir::new().unwrap();
    let environment = FakeEnvironment::with_home(home.path());
    let proxy = MockProxy::start(vec![listing(&[("claude-1", None)])]).await;
    let settings_path = settings_path_for(Target::Claude, &environment).unwrap();
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    fs::write(
        &settings_path,
        r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-old"}}"#,
    )
    .unwrap();
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
    set_base_url(&harness.environment, "http://elsewhere.test");
    harness.unconfigure(unconfigure_claude()).await.unwrap();
    let expected = "env.ANTHROPIC_AUTH_TOKEN (captured with no ANTHROPIC_BASE_URL (Anthropic's default endpoint))";
    let stdout = harness.stdout_text();
    assert!(stdout.contains(expected), "{stdout}");
}

#[tokio::test]
async fn unconfigure_without_a_mismatch_drops_the_receipt() {
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
    harness.unconfigure(unconfigure_claude()).await.unwrap();
    let stdout = harness.stdout_text();
    assert!(!stdout.contains("Left removed, since the file now points"));
    assert!(stdout.contains(&format!(
        "No settings file remains at {}; it held nothing but `lite configure claude`'s own keys.",
        settings_path.display()
    )));
    assert!(!settings_path.exists());
    assert!(!state_path.exists());
}
