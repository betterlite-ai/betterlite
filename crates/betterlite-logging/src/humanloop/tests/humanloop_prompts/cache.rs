use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;

use super::support::{PROMPT_BODY, fake_clock, fill_cache, logger, replying, resolve_settings};
use crate::humanloop::prompt_cache::{Clock as _, MAX_CACHED_PROMPTS, SystemClock};
use crate::humanloop::settings::{DEFAULT_CACHE_TTL_SECONDS, SettingsError};

#[rstest]
#[case::unset(None, 60)]
#[case::empty(Some(""), 60)]
#[case::configured(Some("5"), 5)]
#[case::padded(Some(" 120 "), 120)]
#[case::zero(Some("0"), 0)]
fn cache_ttl_defaults_to_sixty_seconds(#[case] text: Option<&str>, #[case] expected: u64) {
    let settings = resolve_settings(None, text).expect("settings");
    assert_eq!(settings.cache_ttl_seconds, expected);
    assert_eq!(DEFAULT_CACHE_TTL_SECONDS, 60);
}

#[rstest]
#[case::not_a_number("soon")]
#[case::negative("-5")]
fn cache_ttl_that_is_not_an_unsigned_number_is_a_settings_error(#[case] text: &str) {
    let error = resolve_settings(None, Some(text)).expect_err("settings");
    assert!(matches!(error, SettingsError::CacheTtl(_)));
    assert_eq!(
        error.to_string(),
        "environment variable HUMANLOOP_PROMPT_CACHE_TTL_SECONDS must be a number"
    );
}

#[rstest]
#[case::missing(None, false)]
#[case::empty(Some(""), false)]
#[case::blank(Some("  "), false)]
#[case::present(Some("hl-key"), true)]
fn api_key_is_set_only_for_a_non_empty_value(#[case] text: Option<&str>, #[case] present: bool) {
    let settings = resolve_settings(text, None).expect("settings");
    assert_eq!(settings.api_key.is_some(), present);
}

#[tokio::test]
async fn prompt_is_cached_until_the_ttl_has_passed() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"), None);
    let prompts = &logger.manager;
    let key = SecretString::from("hl-key");
    prompts
        .resolve("pr_abc123", &key, |prompt| prompt.clone())
        .await
        .expect("miss");
    clock.advance(60);
    prompts
        .resolve("pr_abc123", &key, |prompt| prompt.clone())
        .await
        .expect("hit");
    assert_eq!(recorded.lock().expect("lock").len(), 1);
    clock.advance(1);
    prompts
        .resolve("pr_abc123", &key, |prompt| prompt.clone())
        .await
        .expect("expired");
    assert_eq!(recorded.lock().expect("lock").len(), 2);
}

#[tokio::test]
async fn the_configured_ttl_governs_the_cache() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"), Some("30"));
    let prompts = &logger.manager;
    let key = SecretString::from("hl-key");
    prompts
        .resolve("pr_abc123", &key, |prompt| prompt.clone())
        .await
        .expect("miss");
    clock.advance(30);
    prompts
        .resolve("pr_abc123", &key, |prompt| prompt.clone())
        .await
        .expect("hit");
    assert_eq!(recorded.lock().expect("lock").len(), 1);
    clock.advance(1);
    prompts
        .resolve("pr_abc123", &key, |prompt| prompt.clone())
        .await
        .expect("expired");
    assert_eq!(recorded.lock().expect("lock").len(), 2);
}

#[tokio::test]
async fn cache_key_is_the_prompt_id_and_not_the_api_key() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"), None);
    let prompts = &logger.manager;
    prompts
        .resolve("pr_abc123", &SecretString::from("first-key"), |prompt| {
            prompt.clone()
        })
        .await
        .expect("first");
    prompts
        .resolve("pr_abc123", &SecretString::from("second-key"), |prompt| {
            prompt.clone()
        })
        .await
        .expect("second");
    assert_eq!(recorded.lock().expect("lock").len(), 1);
}

#[tokio::test]
async fn each_prompt_id_is_fetched_once() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"), None);
    let prompts = &logger.manager;
    let key = SecretString::from("hl-key");
    prompts
        .resolve("pr_one", &key, |prompt| prompt.clone())
        .await
        .expect("one");
    prompts
        .resolve("pr_two", &key, |prompt| prompt.clone())
        .await
        .expect("two");
    prompts
        .resolve("pr_one", &key, |prompt| prompt.clone())
        .await
        .expect("one again");
    assert_eq!(recorded.lock().expect("lock").len(), 2);
}

#[tokio::test]
async fn a_failed_fetch_is_not_cached() {
    let clock = fake_clock();
    let (transport, recorded) = replying(500, "boom");
    let logger = logger(transport, &clock, Some("hl-key"), None);
    let prompts = &logger.manager;
    let key = SecretString::from("hl-key");
    prompts
        .resolve("pr_abc123", &key, |prompt| prompt.clone())
        .await
        .expect_err("first");
    prompts
        .resolve("pr_abc123", &key, |prompt| prompt.clone())
        .await
        .expect_err("second");
    assert_eq!(recorded.lock().expect("lock").len(), 2);
}

#[tokio::test]
async fn the_cache_holds_two_hundred_prompts_and_drops_the_earliest_expiry() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"), Some("100000"));
    let prompts = &logger.manager;
    let key = SecretString::from("hl-key");
    fill_cache(prompts, &key, MAX_CACHED_PROMPTS + 1, Some(1)).await;
    assert_eq!(recorded.lock().expect("lock").len(), MAX_CACHED_PROMPTS + 1);
    assert_eq!(
        prompts.cache.lock().expect("lock").entries.len(),
        MAX_CACHED_PROMPTS
    );
    prompts
        .resolve("pr_1", &key, |prompt| prompt.clone())
        .await
        .expect("kept");
    assert_eq!(recorded.lock().expect("lock").len(), MAX_CACHED_PROMPTS + 1);
    prompts
        .resolve("pr_0", &key, |prompt| prompt.clone())
        .await
        .expect("evicted");
    assert_eq!(recorded.lock().expect("lock").len(), MAX_CACHED_PROMPTS + 2);
}

#[tokio::test]
async fn expired_prompts_are_dropped_before_the_capacity_check() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let logger = logger(transport, &clock, Some("hl-key"), Some("10"));
    let prompts = &logger.manager;
    let key = SecretString::from("hl-key");
    fill_cache(prompts, &key, MAX_CACHED_PROMPTS, None).await;
    clock.advance(11);
    prompts
        .resolve("pr_new", &key, |prompt| prompt.clone())
        .await
        .expect("new");
    assert_eq!(prompts.cache.lock().expect("lock").entries.len(), 1);
    assert_eq!(recorded.lock().expect("lock").len(), MAX_CACHED_PROMPTS + 1);
}

#[test]
fn the_system_clock_reports_time_since_the_epoch() {
    assert!(SystemClock.now() > Duration::ZERO);
}
