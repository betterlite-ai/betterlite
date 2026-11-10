#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use betterlite_config::sections::alert_settings::types::{AlertType, AlertTypeConfig};
use betterlite_config::sections::alert_settings::webhooks::WebhookUrl;
use chrono::{DateTime, FixedOffset, TimeZone as _};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    Clock, DigestAlert, DigestBuckets, DigestState, ExpiredDigest, MAX_DIGEST_ENTRIES, SystemClock,
    digest_queue_items, render_summary,
};
use crate::alert_settings::records::DigestEntry;

#[derive(Debug)]
struct FakeClock {
    now: Mutex<DateTime<FixedOffset>>,
}

impl FakeClock {
    fn new(now: DateTime<FixedOffset>) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }

    fn set(&self, now: DateTime<FixedOffset>) {
        *self.now.lock().unwrap() = now;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<FixedOffset> {
        *self.now.lock().unwrap()
    }
}

fn time(hour: u32, minute: u32, second: u32) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(0)
        .expect("UTC is a valid offset")
        .with_ymd_and_hms(2026, 1, 1, hour, minute, second)
        .single()
        .expect("the test time is unambiguous")
}

fn next_day(hour: u32, minute: u32, second: u32) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(0)
        .expect("UTC is a valid offset")
        .with_ymd_and_hms(2026, 1, 2, hour, minute, second)
        .single()
        .expect("the test time is unambiguous")
}

fn buckets_with(config: BTreeMap<AlertType, AlertTypeConfig>) -> (DigestBuckets, Arc<FakeClock>) {
    let clock = Arc::new(FakeClock::new(time(12, 0, 0)));
    let concrete: Arc<FakeClock> = Arc::clone(&clock);
    let clock_trait: Arc<dyn Clock> = concrete;
    (
        DigestBuckets {
            state: Mutex::new(DigestState {
                config,
                entries: IndexMap::new(),
            }),
            clock: clock_trait,
        },
        clock,
    )
}

fn buckets(types: &[(AlertType, u32)]) -> (DigestBuckets, Arc<FakeClock>) {
    let config = types
        .iter()
        .map(|(alert_type, interval)| {
            (
                *alert_type,
                AlertTypeConfig {
                    digest: true,
                    digest_interval: *interval,
                },
            )
        })
        .collect();
    buckets_with(config)
}

fn url(value: &str) -> WebhookUrl {
    WebhookUrl::Single(SecretString::from(value))
}

fn record(buckets: &DigestBuckets, alert_type: AlertType, message: &str, webhook_url: WebhookUrl) {
    buckets.record(DigestAlert {
        alert_type,
        request_model: None,
        api_base: None,
        message,
        level: "High",
        webhook_url,
    });
}

fn record_keyed(
    buckets: &DigestBuckets,
    request_model: Option<&str>,
    api_base: Option<&str>,
    message: &str,
) {
    buckets.record(DigestAlert {
        alert_type: AlertType::BudgetAlerts,
        request_model,
        api_base,
        message,
        level: "High",
        webhook_url: url("https://hooks/a"),
    });
}

fn expired_digest(webhook_url: WebhookUrl) -> ExpiredDigest {
    ExpiredDigest {
        alert_type: AlertType::BudgetAlerts,
        entry: DigestEntry {
            alert_type: "budget_alerts".to_owned(),
            request_model: String::new(),
            api_base: String::new(),
            first_message: "the budget is close".to_owned(),
            level: "High".to_owned(),
            count: 3,
            start_time: time(12, 0, 0),
            last_time: time(12, 30, 5),
            webhook_url,
        },
    }
}

#[test]
fn the_system_clock_reports_utc() {
    let now = SystemClock.now();
    assert_eq!(now.offset().local_minus_utc(), 0_i32);
}

#[test]
fn digest_mode_is_enabled_only_by_the_matching_alert_type_config() {
    let config = BTreeMap::from([
        (
            AlertType::BudgetAlerts,
            AlertTypeConfig {
                digest: true,
                digest_interval: 60,
            },
        ),
        (
            AlertType::OutageAlerts,
            AlertTypeConfig {
                digest: false,
                digest_interval: 60,
            },
        ),
    ]);
    let (buckets, _clock) = buckets_with(config);
    assert!(buckets.digest_enabled(AlertType::BudgetAlerts));
    assert!(!buckets.digest_enabled(AlertType::OutageAlerts));
    assert!(!buckets.digest_enabled(AlertType::LlmExceptions));
}

#[test]
fn the_alert_type_config_can_be_replaced() {
    let (buckets, _clock) = buckets(&[]);
    assert!(!buckets.digest_enabled(AlertType::BudgetAlerts));
    let config = BTreeMap::from([(
        AlertType::BudgetAlerts,
        AlertTypeConfig {
            digest: true,
            digest_interval: 5,
        },
    )]);
    buckets.set_config(config);
    assert!(buckets.digest_enabled(AlertType::BudgetAlerts));
}

#[test]
fn a_digest_entry_keeps_the_first_message_level_url_and_start_time() {
    let (buckets, clock) = buckets(&[(AlertType::BudgetAlerts, 60)]);
    buckets.record(DigestAlert {
        alert_type: AlertType::BudgetAlerts,
        request_model: Some("gpt-4o"),
        api_base: Some("https://api.example"),
        message: "first message",
        level: "High",
        webhook_url: url("https://hooks/first"),
    });
    clock.set(time(12, 0, 30));
    buckets.record(DigestAlert {
        alert_type: AlertType::BudgetAlerts,
        request_model: Some("gpt-4o"),
        api_base: Some("https://api.example"),
        message: "second message",
        level: "Low",
        webhook_url: url("https://hooks/second"),
    });
    let state = buckets.state.lock().unwrap();
    assert_eq!(state.entries.len(), 1);
    let entry = state
        .entries
        .values()
        .next()
        .expect("the bucket exists")
        .clone();
    drop(state);
    assert_eq!(entry.alert_type, "budget_alerts");
    assert_eq!(entry.request_model, "gpt-4o");
    assert_eq!(entry.api_base, "https://api.example");
    assert_eq!(entry.first_message, "first message");
    assert_eq!(entry.level, "High");
    assert_eq!(entry.count, 2);
    assert_eq!(entry.start_time, time(12, 0, 0));
    assert_eq!(entry.last_time, time(12, 0, 30));
    let WebhookUrl::Single(value) = &entry.webhook_url else {
        panic!("the first webhook URL stays a single URL");
    };
    assert_eq!(value.expose_secret(), "https://hooks/first");
}

#[test]
fn digest_buckets_are_keyed_by_alert_type_request_model_and_api_base() {
    let (buckets, _clock) = buckets(&[(AlertType::BudgetAlerts, 60)]);
    record_keyed(&buckets, None, None, "one");
    record_keyed(&buckets, Some("gpt-4o"), None, "two");
    record_keyed(
        &buckets,
        Some("gpt-4o"),
        Some("https://api.example"),
        "three",
    );
    let keys: Vec<String> = buckets
        .state
        .lock()
        .unwrap()
        .entries
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        keys,
        [
            "budget_alerts::".to_owned(),
            "budget_alerts:gpt-4o:".to_owned(),
            "budget_alerts:gpt-4o:https://api.example".to_owned()
        ]
    );
}

fn fill_to_cap(buckets: &DigestBuckets, alert_type: AlertType) {
    (0..MAX_DIGEST_ENTRIES).for_each(|index| {
        let request_model = format!("model-{index}");
        buckets.record(DigestAlert {
            alert_type,
            request_model: Some(&request_model),
            api_base: None,
            message: "message",
            level: "High",
            webhook_url: url("https://hooks/a"),
        });
    });
}

#[test]
fn a_new_key_beyond_the_digest_cap_folds_into_the_overflow_entry() {
    let (buckets, _clock) = buckets(&[(AlertType::BudgetAlerts, 60)]);
    fill_to_cap(&buckets, AlertType::BudgetAlerts);
    buckets.record(DigestAlert {
        alert_type: AlertType::BudgetAlerts,
        request_model: Some("overflow-model"),
        api_base: None,
        message: "overflow message",
        level: "High",
        webhook_url: url("https://hooks/a"),
    });
    let state = buckets.state.lock().unwrap();
    assert_eq!(state.entries.len(), MAX_DIGEST_ENTRIES + 1);
    assert!(!state.entries.contains_key("budget_alerts:overflow-model:"));
    let overflow = state
        .entries
        .get("budget_alerts::")
        .expect("the overflow entry is created on first use")
        .clone();
    drop(state);
    assert_eq!(overflow.count, 1);
    assert_eq!(overflow.first_message, "overflow message");
    assert_eq!(overflow.request_model, "");
    assert_eq!(overflow.api_base, "");
}

#[test]
fn later_overflow_keys_count_in_the_overflow_entry_of_their_alert_type() {
    let (buckets, _clock) =
        buckets(&[(AlertType::BudgetAlerts, 60), (AlertType::OutageAlerts, 60)]);
    fill_to_cap(&buckets, AlertType::BudgetAlerts);
    record_keyed(&buckets, Some("overflow-one"), None, "one");
    record_keyed(&buckets, Some("overflow-two"), None, "two");
    buckets.record(DigestAlert {
        alert_type: AlertType::OutageAlerts,
        request_model: Some("outage-model"),
        api_base: None,
        message: "outage",
        level: "High",
        webhook_url: url("https://hooks/b"),
    });
    let state = buckets.state.lock().unwrap();
    assert_eq!(state.entries.len(), MAX_DIGEST_ENTRIES + 2);
    let budget = state
        .entries
        .get("budget_alerts::")
        .expect("the budget overflow entry exists")
        .clone();
    let outage = state
        .entries
        .get("outage_alerts::")
        .expect("each alert type gets its own overflow entry")
        .clone();
    drop(state);
    assert_eq!(budget.count, 2);
    assert_eq!(outage.count, 1);
    assert_eq!(outage.alert_type, "outage_alerts");
}

#[test]
fn a_full_digest_map_still_counts_an_existing_key_in_its_own_entry() {
    let (buckets, _clock) = buckets(&[(AlertType::BudgetAlerts, 60)]);
    fill_to_cap(&buckets, AlertType::BudgetAlerts);
    buckets.record(DigestAlert {
        alert_type: AlertType::BudgetAlerts,
        request_model: Some("model-0"),
        api_base: None,
        message: "message",
        level: "High",
        webhook_url: url("https://hooks/a"),
    });
    let state = buckets.state.lock().unwrap();
    assert_eq!(state.entries.len(), MAX_DIGEST_ENTRIES);
    let count = state
        .entries
        .get("budget_alerts:model-0:")
        .expect("the existing bucket keeps its key")
        .count;
    assert!(!state.entries.contains_key("budget_alerts::"));
    drop(state);
    assert_eq!(count, 2);
}

#[test]
fn a_flushed_bucket_frees_a_slot_for_a_new_key() {
    let (buckets, clock) = buckets(&[(AlertType::BudgetAlerts, 60)]);
    fill_to_cap(&buckets, AlertType::BudgetAlerts);
    clock.set(time(12, 1, 0));
    assert_eq!(buckets.flush_expired().len(), MAX_DIGEST_ENTRIES);
    buckets.record(DigestAlert {
        alert_type: AlertType::BudgetAlerts,
        request_model: Some("fresh-model"),
        api_base: None,
        message: "fresh",
        level: "High",
        webhook_url: url("https://hooks/a"),
    });
    let state = buckets.state.lock().unwrap();
    assert_eq!(state.entries.len(), 1);
    assert!(state.entries.contains_key("budget_alerts:fresh-model:"));
    assert!(!state.entries.contains_key("budget_alerts::"));
    drop(state);
}

#[test]
fn a_bucket_flushes_at_its_interval_and_not_before() {
    let (buckets, clock) = buckets(&[(AlertType::BudgetAlerts, 60)]);
    record(
        &buckets,
        AlertType::BudgetAlerts,
        "message",
        url("https://hooks/a"),
    );
    clock.set(time(12, 0, 59));
    assert!(buckets.flush_expired().is_empty());
    clock.set(time(12, 1, 0));
    let expired = buckets.flush_expired();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].alert_type, AlertType::BudgetAlerts);
    assert!(buckets.state.lock().unwrap().entries.is_empty());
}

#[test]
fn a_bucket_without_an_alert_type_config_is_never_flushed() {
    let (buckets, clock) = buckets(&[]);
    record(
        &buckets,
        AlertType::BudgetAlerts,
        "message",
        url("https://hooks/a"),
    );
    clock.set(next_day(12, 0, 1));
    assert!(buckets.flush_expired().is_empty());
    assert_eq!(buckets.state.lock().unwrap().entries.len(), 1);
}

#[test]
fn flushing_removes_only_the_expired_buckets() {
    let (buckets, clock) = buckets(&[
        (AlertType::BudgetAlerts, 60),
        (AlertType::OutageAlerts, 3600),
    ]);
    record(
        &buckets,
        AlertType::BudgetAlerts,
        "budget",
        url("https://hooks/a"),
    );
    record(
        &buckets,
        AlertType::OutageAlerts,
        "outage",
        url("https://hooks/b"),
    );
    clock.set(time(12, 1, 0));
    let expired = buckets.flush_expired();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].alert_type, AlertType::BudgetAlerts);
    let state = buckets.state.lock().unwrap();
    assert_eq!(state.entries.len(), 1);
    assert_eq!(
        state.entries.keys().next().map(String::as_str),
        Some("outage_alerts::")
    );
}

const SUMMARY: &str = "Alert type: `budget_alerts` (Digest)\nLevel: `High`\nStart: `2026-01-01 12:00:00`\nEnd: `2026-01-01 12:30:05`\nCount: `3`\n\nMessage: the budget is close";

#[test]
fn the_digest_summary_names_the_type_level_window_count_and_message() {
    let digest = expired_digest(url("https://hooks/a"));
    assert_eq!(render_summary(&digest, None), SUMMARY);
}

#[rstest]
#[case::set(
    Some("https://proxy.example"),
    "\n\nProxy URL: `https://proxy.example`"
)]
#[case::empty(Some(""), "\n\nProxy URL: ``")]
#[case::unset(None, "")]
fn the_digest_summary_appends_the_proxy_url_when_set(
    #[case] proxy_base_url: Option<&str>,
    #[case] footer: &str,
) {
    let digest = expired_digest(url("https://hooks/a"));
    assert_eq!(
        render_summary(&digest, proxy_base_url),
        format!("{SUMMARY}{footer}")
    );
}

#[test]
fn a_single_webhook_url_becomes_one_queue_item() {
    let expired = vec![expired_digest(url("https://hooks/a"))];
    let items = digest_queue_items(&expired, None);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].url.expose_secret(), "https://hooks/a");
    assert_eq!(
        items[0].headers.get("Content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(items[0].alert_type, AlertType::BudgetAlerts);
    assert!(items[0].format.is_none());
    assert_eq!(items[0].payload.text, render_summary(&expired[0], None));
}

#[test]
fn a_list_of_webhook_urls_becomes_one_queue_item_per_url() {
    let expired = vec![expired_digest(WebhookUrl::Multiple(vec![
        SecretString::from("https://hooks/a"),
        SecretString::from("https://hooks/b"),
    ]))];
    let items = digest_queue_items(&expired, Some("https://proxy.example"));
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].url.expose_secret(), "https://hooks/a");
    assert_eq!(items[1].url.expose_secret(), "https://hooks/b");
    assert_eq!(items[0].payload.text, items[1].payload.text);
    assert!(
        items[0]
            .payload
            .text
            .contains("Proxy URL: `https://proxy.example`")
    );
}
