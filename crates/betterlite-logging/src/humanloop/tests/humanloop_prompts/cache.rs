use std::time::Duration;

use rstest::rstest;
use secrecy::SecretString;

use super::support::{PROMPT_BODY, fake_clock, fill_cache, manager, replying};
use crate::humanloop::prompt_cache::{Clock as _, MAX_CACHED_PROMPTS, SystemClock};
use crate::humanloop::settings::{DEFAULT_CACHE_TTL_SECONDS, HumanloopSettings, SettingsError};

#[rstest]
#[case::unset(None, 60)]
#[case::empty(Some(""), 60)]
#[case::configured(Some("5"), 5)]
#[case::padded(Some(" 120 "), 120)]
#[case::zero(Some("0"), 0)]
fn cache_ttl_defaults_to_sixty_seconds(#[case] text: Option<&str>, #[case] expected: u64) {
    let settings = HumanloopSettings::from_env_values(None, text).expect("settings");
    assert_eq!(settings.cache_ttl_seconds, expected);
    assert_eq!(DEFAULT_CACHE_TTL_SECONDS, 60);
}

#[rstest]
#[case::not_a_number("soon")]
#[case::negative("-5")]
fn cache_ttl_that_is_not_an_unsigned_number_is_a_settings_error(#[case] text: &str) {
    let error = HumanloopSettings::from_env_values(None, Some(text)).expect_err("settings");
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
    let settings = HumanloopSettings::from_env_values(text, None).expect("settings");
    assert_eq!(settings.api_key.is_some(), present);
}

#[test]
fn prompt_is_cached_until_the_ttl_has_passed() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let mut prompts = manager(transport, &clock, 60);
    let key = SecretString::from("hl-key");
    prompts.get_prompt_from_id("pr_abc123", &key).expect("miss");
    clock.advance(60);
    prompts.get_prompt_from_id("pr_abc123", &key).expect("hit");
    assert_eq!(recorded.lock().expect("lock").len(), 1);
    clock.advance(1);
    prompts
        .get_prompt_from_id("pr_abc123", &key)
        .expect("expired");
    assert_eq!(recorded.lock().expect("lock").len(), 2);
}

#[test]
fn the_configured_ttl_governs_the_cache() {
    let clock = fake_clock();
    let settings = HumanloopSettings::from_env_values(None, Some("30")).expect("settings");
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let mut prompts = manager(transport, &clock, settings.cache_ttl_seconds);
    let key = SecretString::from("hl-key");
    prompts.get_prompt_from_id("pr_abc123", &key).expect("miss");
    clock.advance(30);
    prompts.get_prompt_from_id("pr_abc123", &key).expect("hit");
    assert_eq!(recorded.lock().expect("lock").len(), 1);
    clock.advance(1);
    prompts
        .get_prompt_from_id("pr_abc123", &key)
        .expect("expired");
    assert_eq!(recorded.lock().expect("lock").len(), 2);
}

#[test]
fn cache_key_is_the_prompt_id_and_not_the_api_key() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let mut prompts = manager(transport, &clock, 60);
    prompts
        .get_prompt_from_id("pr_abc123", &SecretString::from("first-key"))
        .expect("first");
    prompts
        .get_prompt_from_id("pr_abc123", &SecretString::from("second-key"))
        .expect("second");
    assert_eq!(recorded.lock().expect("lock").len(), 1);
}

#[test]
fn each_prompt_id_is_fetched_once() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let mut prompts = manager(transport, &clock, 60);
    let key = SecretString::from("hl-key");
    prompts.get_prompt_from_id("pr_one", &key).expect("one");
    prompts.get_prompt_from_id("pr_two", &key).expect("two");
    prompts
        .get_prompt_from_id("pr_one", &key)
        .expect("one again");
    assert_eq!(recorded.lock().expect("lock").len(), 2);
}

#[test]
fn a_failed_fetch_is_not_cached() {
    let clock = fake_clock();
    let (transport, recorded) = replying(500, "boom");
    let mut prompts = manager(transport, &clock, 60);
    let key = SecretString::from("hl-key");
    prompts
        .get_prompt_from_id("pr_abc123", &key)
        .expect_err("first");
    prompts
        .get_prompt_from_id("pr_abc123", &key)
        .expect_err("second");
    assert_eq!(recorded.lock().expect("lock").len(), 2);
}

#[test]
fn the_cache_holds_two_hundred_prompts_and_drops_the_earliest_expiry() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let mut prompts = manager(transport, &clock, 100_000);
    let key = SecretString::from("hl-key");
    fill_cache(&mut prompts, &key, MAX_CACHED_PROMPTS + 1, Some(1));
    assert_eq!(recorded.lock().expect("lock").len(), MAX_CACHED_PROMPTS + 1);
    assert_eq!(prompts.cache.entries.len(), MAX_CACHED_PROMPTS);
    prompts.get_prompt_from_id("pr_1", &key).expect("kept");
    assert_eq!(recorded.lock().expect("lock").len(), MAX_CACHED_PROMPTS + 1);
    prompts.get_prompt_from_id("pr_0", &key).expect("evicted");
    assert_eq!(recorded.lock().expect("lock").len(), MAX_CACHED_PROMPTS + 2);
}

#[test]
fn expired_prompts_are_dropped_before_the_capacity_check() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let mut prompts = manager(transport, &clock, 10);
    let key = SecretString::from("hl-key");
    fill_cache(&mut prompts, &key, MAX_CACHED_PROMPTS, None);
    clock.advance(11);
    prompts.get_prompt_from_id("pr_new", &key).expect("new");
    assert_eq!(prompts.cache.entries.len(), 1);
    assert_eq!(recorded.lock().expect("lock").len(), MAX_CACHED_PROMPTS + 1);
}

#[test]
fn a_failed_fetch_at_capacity_still_evicts_the_earliest_entry() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let mut prompts = manager(transport, &clock, 100_000);
    let key = SecretString::from("hl-key");
    fill_cache(&mut prompts, &key, MAX_CACHED_PROMPTS, Some(1));
    assert_eq!(prompts.cache.entries.len(), MAX_CACHED_PROMPTS);
    prompts.transport.status = None;
    prompts
        .get_prompt_from_id("pr_missing", &key)
        .expect_err("fetch");
    assert_eq!(prompts.cache.entries.len(), MAX_CACHED_PROMPTS - 1);
    assert_eq!(recorded.lock().expect("lock").len(), MAX_CACHED_PROMPTS + 1);
}

#[test]
fn the_system_clock_reports_time_since_the_epoch() {
    assert!(SystemClock.now() > Duration::ZERO);
}
