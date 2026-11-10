use std::time::Duration;

use rstest::rstest;

use super::support::{Logs, redis_cache};
use crate::config_sync::channel::{CONFIG_SYNC_CHANNEL, config_sync_channel};
use crate::config_sync::publish::{publish_config_change, publish_config_param_change};
use crate::config_sync::settings::ConfigSyncSettings;
use crate::redis_live_client::tests::fake_server::{FakeServer, Reply};

#[test]
fn the_config_sync_channel_is_litellms_channel_name() {
    assert_eq!(CONFIG_SYNC_CHANNEL, "litellm_proxy.config_change");
    assert_eq!(config_sync_channel(None), "litellm_proxy.config_change");
    assert_eq!(
        config_sync_channel(Some("blue")),
        "blue:litellm_proxy.config_change"
    );
}

#[test]
fn the_config_sync_defaults_match_litellm() {
    let settings = ConfigSyncSettings::default();
    assert_eq!(settings.debounce, Duration::from_secs(1));
    assert_eq!(settings.jitter_max_nanos, 5_000_000_000);
    assert_eq!(settings.min_resync_interval, Duration::from_secs(10));
    assert_eq!(settings.poll_timeout, Duration::from_secs(1));
    assert_eq!(settings.backoff_initial, Duration::from_secs(5));
    assert_eq!(settings.backoff_max, Duration::from_secs(60));
}

#[tokio::test]
async fn config_change_publishes_the_object_type_on_the_default_channel() {
    let server = FakeServer::start().await;
    server.script("PUBLISH", Reply::Int(1));
    let cache = redis_cache(&server, None).await;
    publish_config_change(Some(&cache), "litellm_proxymodeltable").await;
    let published = server.commands_named("PUBLISH");
    assert_eq!(published.len(), 1);
    assert_eq!(
        published[0].args,
        [
            "PUBLISH",
            CONFIG_SYNC_CHANNEL,
            r#"{"object_type":"litellm_proxymodeltable"}"#,
        ]
    );
}

#[rstest]
#[case::plain("blue", "blue:litellm_proxy.config_change")]
#[case::empty("", ":litellm_proxy.config_change")]
#[case::nested("team:blue", "team:blue:litellm_proxy.config_change")]
#[tokio::test]
async fn config_change_prefixes_the_channel_with_the_namespace(
    #[case] namespace: &'static str,
    #[case] channel: &'static str,
) {
    let server = FakeServer::start().await;
    server.script("PUBLISH", Reply::Int(1));
    let cache = redis_cache(&server, Some(namespace)).await;
    publish_config_change(Some(&cache), "litellm_agentstable").await;
    let published = server.commands_named("PUBLISH");
    assert_eq!(published.len(), 1);
    assert_eq!(
        published[0].args,
        [
            "PUBLISH",
            channel,
            r#"{"object_type":"litellm_agentstable"}"#,
        ]
    );
}

#[tokio::test]
async fn publishing_without_a_coordination_cache_is_a_no_op() {
    let (logs, _guard) = Logs::capture();
    publish_config_change(None, "litellm_prompttable").await;
    assert!(logs.lines().is_empty());
}

#[tokio::test]
async fn a_publish_error_is_logged_and_swallowed() {
    let (logs, _guard) = Logs::capture();
    let server = FakeServer::start().await;
    server.script(
        "PUBLISH",
        Reply::Error("READONLY you cannot write against a read only replica".to_owned()),
    );
    let cache = redis_cache(&server, None).await;
    publish_config_change(Some(&cache), "litellm_prompttable").await;
    let expected = "config sync publish for litellm_prompttable failed: ";
    assert!(
        logs.lines().iter().any(|line| line.contains(expected)),
        "a failed publish logs a warning: {:?}",
        logs.lines()
    );
}

#[tokio::test]
async fn a_publish_to_an_unreachable_redis_is_logged_and_swallowed() {
    let (logs, _guard) = Logs::capture();
    let server = FakeServer::start().await;
    let cache = redis_cache(&server, None).await;
    drop(server);
    publish_config_change(Some(&cache), "litellm_uisettings").await;
    let expected = "config sync publish for litellm_uisettings failed: ";
    assert!(
        logs.lines().iter().any(|line| line.contains(expected)),
        "an unreachable redis logs a warning: {:?}",
        logs.lines()
    );
}

#[rstest]
#[case::general("general_settings")]
#[case::router("router_settings")]
#[case::litellm("litellm_settings")]
#[case::cost_map("model_cost_map_reload_config")]
#[case::anthropic_beta("anthropic_beta_headers_reload_config")]
#[tokio::test]
async fn config_param_changes_publish_the_param_name(#[case] param: &'static str) {
    let server = FakeServer::start().await;
    server.script("PUBLISH", Reply::Int(1));
    let cache = redis_cache(&server, None).await;
    publish_config_param_change(Some(&cache), param).await;
    let published = server.commands_named("PUBLISH");
    assert_eq!(published.len(), 1);
    let payload = format!(r#"{{"object_type":"{param}"}}"#);
    assert_eq!(
        published[0].args,
        ["PUBLISH", CONFIG_SYNC_CHANNEL, payload.as_str()]
    );
}

#[tokio::test]
async fn params_without_a_resync_callback_do_not_publish() {
    let (logs, _guard) = Logs::capture();
    let server = FakeServer::start().await;
    let cache = redis_cache(&server, None).await;
    publish_config_param_change(Some(&cache), "custom_param").await;
    assert!(server.commands_named("PUBLISH").is_empty());
    let expected = "config sync publish for custom_param skipped";
    assert!(
        logs.lines().iter().any(|line| line.contains(expected)),
        "a skipped param logs a debug line: {:?}",
        logs.lines()
    );
}
