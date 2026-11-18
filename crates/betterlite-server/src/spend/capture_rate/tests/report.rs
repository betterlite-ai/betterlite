use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::capture_rate::report::CheckResult;
use secrecy::ExposeSecret as _;

use super::super::{CaptureRateJobError, run_check};
use super::credentials::SecretBillingCredentials;
use super::*;

#[tokio::test]
async fn a_missing_credential_skips_billing_and_captured_spend() {
    let timeline = Arc::new(Timeline::default());
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
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 3, &[]);
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert!(matches!(
        results.as_slice(),
        [CheckResult::CredentialMissing { env_var, .. }] if env_var == "OPENAI_ADMIN_KEY"
    ));
    assert_eq!(publisher.take(), vec![(SpendCaptureProvider::Openai, None)]);
    assert_eq!(
        alert.take(),
        vec![
            "Spend capture-rate check: OPENAI_ADMIN_KEY is not set, so the openai bill cannot be read. Set it or remove general_settings.spend_capture_rate_check. https://docs.litellm.ai/docs/proxy/spend_capture_rate"
                .to_owned()
        ]
    );
    assert!(http.take_requests().is_empty());
    assert!(spend.take_requests().is_empty());
    assert_eq!(timeline.take(), vec!["publish", "alert"]);
}

#[tokio::test]
async fn a_failed_bill_skips_the_captured_spend_query() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![Ok(BillingHttpResponse {
        status: 500,
        body: b"boom".to_vec(),
    })]);
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
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 3, &[]);
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert!(matches!(
        results.as_slice(),
        [CheckResult::RequestFailed { detail, .. }] if detail == "HTTP 500: boom"
    ));
    assert_eq!(publisher.take(), vec![(SpendCaptureProvider::Openai, None)]);
    assert_eq!(
        alert.take(),
        vec![
            "Spend capture-rate check: could not read the openai bill (HTTP 500: boom). https://docs.litellm.ai/docs/proxy/spend_capture_rate"
                .to_owned()
        ]
    );
    assert!(spend.take_requests().is_empty());
}

#[tokio::test]
async fn a_transport_failure_becomes_a_request_failed_result() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![Err(BillingTransportError::Transport(
        TransportError::Closed,
    ))]);
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
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 3, &[]);
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    let [CheckResult::RequestFailed { detail, .. }] = results.as_slice() else {
        panic!("expected a request failure");
    };
    assert!(detail.starts_with("request failed: "), "{detail}");
    assert!(spend.take_requests().is_empty());
}

#[tokio::test]
async fn a_report_covers_yesterday_and_the_preceding_lookback_days() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-16", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-16", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 3, &["proj_a"]);
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    let [CheckResult::Report(report)] = results.as_slice() else {
        panic!("expected a report");
    };
    assert_eq!(report.provider, SpendCaptureProvider::Openai);
    assert_eq!(report.start_date, "2026-11-15");
    assert_eq!(report.end_date, "2026-11-17");
    assert_eq!(report.days.len(), 3);
    assert_eq!(report.captured_spend, dec("45"));
    assert_eq!(report.provider_spend, dec("100"));
    assert_eq!(report.capture_rate, Some(dec("0.45")));
    assert!(report.below_threshold);
    assert_eq!(
        publisher.take(),
        vec![(SpendCaptureProvider::Openai, Some(dec("0.45")))]
    );
    assert_eq!(
        alert.take(),
        vec![
            "Spend capture rate for openai is 45.0%, under the 90% threshold: LiteLLM captured $45.00 of the $100.00 openai bill for 2026-11-15 to 2026-11-17. Requests reach openai outside LiteLLM or cost tracking is dropping spend. https://docs.litellm.ai/docs/proxy/spend_capture_rate"
                .to_owned()
        ]
    );
    assert_eq!(timeline.take(), vec!["publish", "alert"]);
    let calls = spend.take_requests();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].providers,
        vec!["openai".to_owned(), "text-completion-openai".to_owned()]
    );
    assert_eq!(calls[0].start_date, date("2026-11-15"));
    assert_eq!(calls[0].end_date, date("2026-11-17"));
    let requests = http.take_requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .query
            .iter()
            .any(|(name, value)| name == "project_ids[]" && value == "proj_a")
    );
    assert!(
        requests[0]
            .query
            .iter()
            .any(|(name, value)| name == "start_time"
                && *value == midnight("2026-11-15").to_string())
    );
    assert!(
        requests[0]
            .query
            .iter()
            .any(|(name, value)| name == "end_time" && *value == midnight("2026-11-18").to_string())
    );
}

#[tokio::test]
async fn a_healthy_report_publishes_its_rate_without_alerting() {
    let timeline = Arc::new(Timeline::default());
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
    run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(
        publisher.take(),
        vec![(SpendCaptureProvider::Openai, Some(dec("0.95")))]
    );
    assert!(alert.take().is_empty());
    assert_eq!(timeline.take(), vec!["publish"]);
}

#[tokio::test]
async fn a_zero_bill_publishes_no_ratio_and_does_not_alert() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(empty_page())]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "5")]))]);
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
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    let [CheckResult::Report(report)] = results.as_slice() else {
        panic!("expected a report");
    };
    assert_eq!(report.capture_rate, None);
    assert!(!report.below_threshold);
    assert_eq!(publisher.take(), vec![(SpendCaptureProvider::Openai, None)]);
    assert!(alert.take().is_empty());
}

#[tokio::test]
async fn duplicate_providers_are_checked_and_published_twice() {
    let timeline = Arc::new(Timeline::default());
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
    let settings = settings(
        &[SpendCaptureProvider::Openai, SpendCaptureProvider::Openai],
        "0.9",
        1,
        &[],
    );
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(
        publisher.take(),
        vec![
            (SpendCaptureProvider::Openai, None),
            (SpendCaptureProvider::Openai, None)
        ]
    );
    assert_eq!(alert.take().len(), 2);
    assert_eq!(
        timeline.take(),
        vec!["publish", "publish", "alert", "alert"]
    );
}

#[tokio::test]
async fn a_later_failure_interrupts_before_any_publication() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![
        ok_page(page("2026-11-17", "100")),
        ok_page(page("2026-11-17", "100")),
    ]);
    let unreadable = CapturedSpendError::UnreadableAmount {
        date: "2026-11-17".to_owned(),
        source: "not-a-decimal".parse::<Decimal>().unwrap_err(),
    };
    let spend = FakeSpend::with(vec![
        Ok(spend_days(&[("2026-11-17", "45")])),
        Err(unreadable),
    ]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(
        &[SpendCaptureProvider::Openai, SpendCaptureProvider::Openai],
        "0.9",
        1,
        &[],
    );
    let error = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap_err();
    assert!(matches!(error, CaptureRateJobError::Spend(_)));
    assert!(publisher.take().is_empty());
    assert!(alert.take().is_empty());
    assert!(timeline.take().is_empty());
    assert_eq!(http.take_requests().len(), 2);
}

#[tokio::test]
async fn a_lookback_below_one_is_an_error_before_any_query() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
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
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 0, &[]);
    let error = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap_err();
    assert!(matches!(error, CaptureRateJobError::Lookback));
    assert!(publisher.take().is_empty());
    assert!(http.take_requests().is_empty());
    assert!(spend.take_requests().is_empty());
}

#[tokio::test]
async fn an_alert_failure_propagates_after_the_publication() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::failing(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    let error = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap_err();
    assert!(matches!(error, CaptureRateJobError::Alert(_)));
    assert_eq!(publisher.take().len(), 1);
    assert_eq!(alert.take().len(), 1);
    assert_eq!(timeline.take(), vec!["publish", "alert"]);
}
#[tokio::test]
async fn reads_the_admin_key_through_the_secret_lookup() {
    let lookup = secret_lookup();
    let present = BTreeMap::from([("OPENAI_ADMIN_KEY".to_owned(), "sk-admin".to_owned())]);
    let adapter = SecretBillingCredentials {
        lookup: &lookup,
        environment: &present,
    };
    let key = adapter.admin_key().unwrap().unwrap();
    assert_eq!(key.expose_secret(), "sk-admin");
    let absent = BTreeMap::new();
    let adapter = SecretBillingCredentials {
        lookup: &lookup,
        environment: &absent,
    };
    assert!(adapter.admin_key().unwrap().is_none());
    let boolean = BTreeMap::from([("OPENAI_ADMIN_KEY".to_owned(), "true".to_owned())]);
    let adapter = SecretBillingCredentials {
        lookup: &lookup,
        environment: &boolean,
    };
    assert!(adapter.admin_key().unwrap().is_none());
}
