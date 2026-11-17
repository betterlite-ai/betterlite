#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::support::{
    answering_caller, inputs, manager, pull, pull_key, pull_param, record_text, report_of, settled,
    written_record,
};

#[tokio::test]
async fn reuse_refreshes_the_profile_email_from_github() {
    let counted = Arc::new(AtomicUsize::new(0));
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.complete = answering_caller(Some(Arc::clone(&counted)));
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    inputs.repository = inputs
        .repository
        .with_param(&pull_param(&key), &record_text(12, Some(key.as_str())));
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    let record = &report.pulls[0];
    assert_eq!(record.profile_email, "dev@example.com");
    assert_eq!(
        record.emails,
        vec!["dev@example.com".to_owned(), "work@example.com".to_owned()]
    );
    let stored = written_record(&inputs.repository, &pull_param(&key));
    assert_eq!(stored.profile_email, "dev@example.com");
    assert_eq!(stored.cache_key.as_deref(), Some(key.as_str()));
    assert_eq!(counted.load(Ordering::SeqCst), 0);
    assert_eq!(record.estimate.cached, Some(true));
}

#[tokio::test]
async fn reuse_falls_back_to_the_cached_profile_email() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    inputs.routes = inputs.routes.clone().with("/users/dev", 404, "");
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    inputs.repository = inputs
        .repository
        .with_param(&pull_param(&key), &record_text(12, Some(key.as_str())));
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    let record = &report.pulls[0];
    assert_eq!(record.profile_email, "old@example.com");
    assert_eq!(
        record.emails,
        vec!["old@example.com".to_owned(), "work@example.com".to_owned()]
    );
    assert_eq!(record.estimate.cached, Some(true));
}

#[tokio::test]
async fn reuse_merges_commit_emails_without_duplicates() {
    let manager = manager(None);
    let mut inputs = inputs("o/r", &[12]);
    let key = pull_key(&inputs.settings, "o/r", &pull(12)).await;
    let stale = record_text(12, Some(key.as_str())).replace(
        "\"commit_emails\":[\"work@example.com\"]",
        "\"commit_emails\":[\"dev@example.com\",\"\",\"work@example.com\"]",
    );
    inputs.repository = inputs.repository.with_param(&pull_param(&key), &stale);
    assert!(manager.start(inputs.run()).await);
    settled(&manager).await;
    let report = report_of(&inputs.repository);
    let record = &report.pulls[0];
    assert_eq!(
        record.emails,
        vec!["dev@example.com".to_owned(), "work@example.com".to_owned()]
    );
    assert_eq!(
        record.commit_emails.as_deref().map(<[String]>::len),
        Some(3),
        "the cached commit list keeps every entry it had"
    );
}
