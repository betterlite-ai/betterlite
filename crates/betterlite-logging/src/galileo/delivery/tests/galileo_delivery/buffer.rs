use super::super::super::config::MAX_IN_MEMORY_RECORDS;
use super::{API_KEY_ENV, capture_logs, delivery, fill, labels};

#[test]
fn the_cap_is_one_thousand_records() {
    assert_eq!(MAX_IN_MEMORY_RECORDS, 1000);
}

#[test]
fn records_below_the_cap_are_kept_without_a_warning() {
    let (logs, _guard) = capture_logs();
    let delivery = delivery(API_KEY_ENV);
    fill(&delivery, MAX_IN_MEMORY_RECORDS);
    assert_eq!(labels(&delivery).len(), MAX_IN_MEMORY_RECORDS);
    assert!(!logs.text().contains("WARN"), "{}", logs.text());
}

#[test]
fn the_oldest_records_are_dropped_past_the_cap_with_a_warning() {
    let (logs, _guard) = capture_logs();
    let delivery = delivery(API_KEY_ENV);
    fill(&delivery, MAX_IN_MEMORY_RECORDS + 1);
    let kept = labels(&delivery);
    assert_eq!(kept.len(), MAX_IN_MEMORY_RECORDS);
    assert_eq!(kept.first().map(String::as_str), Some("1"));
    assert_eq!(kept.last().map(String::as_str), Some("1000"));
    let text = logs.text();
    assert!(text.contains("WARN"), "{text}");
    assert!(text.contains("dropped 1 oldest record(s)"), "{text}");
    assert!(
        text.contains("Check Galileo connectivity/credentials."),
        "{text}"
    );
}
