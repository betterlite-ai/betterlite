use betterlite_db::queues::pod_lock::owner::lease_key;
use betterlite_models::capture_rate::CHECK_JOB_ID;

use super::super::{CaptureRateJob, run_scheduled_check};
use super::*;

#[tokio::test]
async fn an_absent_lock_manager_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    let no_manager: Option<&PodLockManager<FakeLeaseCache>> = None;
    run_scheduled_check(&job, &settings, no_manager, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(publisher.take().len(), 1);
    assert_eq!(alert.take().len(), 1);
}

#[tokio::test]
async fn an_absent_cache_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let manager = pod_lock_manager(None);
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(alert.take().len(), 1);
    assert_eq!(timeline.take(), vec!["publish", "alert"]);
}

#[tokio::test]
async fn a_held_lock_publishes_then_suppresses_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::held("other-pod", Arc::clone(&timeline)));
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(publisher.take().len(), 1);
    assert!(alert.take().is_empty());
    assert_eq!(cache.set_ttls(), vec![900]);
    assert_eq!(cache.deletes(), 0);
    assert_eq!(
        cache.keys(),
        vec![
            lease_key(CHECK_JOB_ID),
            lease_key(CHECK_JOB_ID),
            lease_key(CHECK_JOB_ID),
        ]
    );
    assert_eq!(
        timeline.take(),
        vec!["publish", "set_owner_if_absent", "get_owner", "get_owner"]
    );
}

#[tokio::test]
async fn a_vanished_lock_after_a_failed_acquisition_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::missing(Arc::clone(&timeline)));
    cache.fail_set();
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(alert.take().len(), 1);
    assert_eq!(cache.deletes(), 0);
    assert_eq!(
        timeline.take(),
        vec!["publish", "set_owner_if_absent", "get_owner", "alert"]
    );
}

#[tokio::test]
async fn an_unreadable_lock_after_a_failed_acquisition_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::held("other-pod", Arc::clone(&timeline)));
    cache.fail_set();
    cache.fail_get();
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(alert.take().len(), 1);
    assert_eq!(cache.deletes(), 0);
}

#[tokio::test]
async fn a_reentrant_owner_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::held("other-pod", Arc::clone(&timeline)));
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    cache.hold(manager.pod_id.as_ref());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(alert.take().len(), 1);
    assert_eq!(cache.deletes(), 0);
    assert_eq!(
        timeline.take(),
        vec!["publish", "set_owner_if_absent", "get_owner", "alert"]
    );
}

#[tokio::test]
async fn a_healthy_scheduled_check_does_not_claim_the_window() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::missing(Arc::clone(&timeline)));
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "95")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(publisher.take().len(), 1);
    assert!(alert.take().is_empty());
    assert!(cache.set_ttls().is_empty());
    assert!(cache.keys().is_empty());
    assert_eq!(timeline.take(), vec!["publish"]);
}
#[tokio::test]
async fn the_scheduled_check_still_publishes_without_a_credential() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::missing(Arc::clone(&timeline)));
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials { key: None };
    let http = FakeHttpGet::with(Vec::new());
    let spend = FakeSpend::with(Vec::new());
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(publisher.take(), vec![(SpendCaptureProvider::Openai, None)]);
    assert_eq!(alert.take().len(), 1);
    assert_eq!(cache.set_ttls(), vec![900]);
    assert_eq!(
        timeline.take(),
        vec!["publish", "set_owner_if_absent", "alert"]
    );
}
