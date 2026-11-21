use std::time::Duration;

use betterlite_config::tls::Verification;

use super::harness::{
    DialOutcome, HarnessOptions, SourceOutcome, harness, invalid_status, invalid_status_code, os,
    websocket,
};
use crate::api::realtime_backend_transport::dial::{RealtimeSsl, realtime_ssl};
use crate::api::realtime_backend_transport::error::RealtimeConnectionError;

#[tokio::test(start_paused = true)]
async fn realtime_backend_open_retry() {
    retries_after_two_hung_handshakes().await;
    reports_the_last_dial_error().await;
    refuses_with_the_status_close().await;
    reports_the_invalid_status_code().await;
    uses_a_provider_owned_backend().await;
    fails_config_before_dialing().await;
    dials_without_verification_for_wss().await;
    keeps_the_ssl_argument_on_a_ws_url();
    closes_after_a_failed_setup_send().await;
    reports_a_failed_backend_close().await;
    replaces_a_send_error_with_the_close_error().await;
}

async fn retries_after_two_hung_handshakes() {
    let mut options = HarnessOptions::default();
    options.dials = vec![DialOutcome::Hang, DialOutcome::Hang, DialOutcome::Connect];
    options.requires_setup = true;
    options.setup = Some("{\"setup\":{}}".to_owned());
    let mut retry = harness(options, Vec::new());
    retry.run().await.unwrap();
    let dials = retry.dials();
    assert_eq!(dials.len(), 3);
    assert!(dials.iter().all(|dial| {
        dial.url == "wss://upstream.test/realtime"
            && dial.max_size == 1024
            && dial.open_timeout == Duration::from_secs(8)
            && dial.ssl == RealtimeSsl::Shared(Verification::Enabled)
    }));
    assert_eq!(dials[1].at - dials[0].at, Duration::from_secs(8));
    assert_eq!(dials[2].at - dials[1].at, Duration::from_secs(8));
    assert_eq!(retry.sent().len(), 1);
    assert!(retry.closed());
    assert_eq!(retry.close(), None);
    assert!(matches!(
        &retry.observation.lock().unwrap().source_outcome,
        SourceOutcome::Closed {
            code: Some(1000),
            ..
        }
    ));
}

async fn reports_the_last_dial_error() {
    let mut options = HarnessOptions::default();
    options.dials = vec![os("first"), websocket("second"), os("third")];
    let mut failing = harness(options, Vec::new());
    let error = failing.run().await.unwrap_err();
    assert_eq!(error.to_string(), "third");
    assert_eq!(failing.dials().len(), 3);
}

async fn refuses_with_the_status_close() {
    let mut options = HarnessOptions::default();
    options.dials = vec![invalid_status(401)];
    let mut rejected = harness(options, Vec::new());
    rejected.run().await.unwrap();
    assert_eq!(rejected.dials().len(), 1);
    assert_eq!(rejected.close().map(|close| close.0), Some(1008));
}

async fn reports_the_invalid_status_code() {
    let mut options = HarnessOptions::default();
    options.dials = vec![invalid_status_code(
        403,
        "server rejected WebSocket connection: HTTP 403",
    )];
    let mut older = harness(options, Vec::new());
    older.run().await.unwrap();
    assert_eq!(older.dials().len(), 1);
    let close = older.close().unwrap();
    assert_eq!(close.0, 1011);
    assert_eq!(
        close.1.as_deref(),
        Some("Internal server error: server rejected WebSocket connection: HTTP 403")
    );
}

async fn uses_a_provider_owned_backend() {
    let mut options = HarnessOptions::default();
    options.own_backend = true;
    let mut provider_owned = harness(options, Vec::new());
    provider_owned.run().await.unwrap();
    assert!(provider_owned.dials().is_empty());
    assert!(provider_owned.closed());
}

async fn fails_config_before_dialing() {
    let mut options = HarnessOptions::default();
    options.validate_error = Some("the api key is missing".to_owned());
    let mut invalid_headers = harness(options, Vec::new());
    let error = invalid_headers.run().await.unwrap_err();
    assert!(matches!(error, RealtimeConnectionError::Config(_)));
    assert!(invalid_headers.dials().is_empty());
    assert!(invalid_headers.texts().is_empty());
    assert!(invalid_headers.close().is_none());
}

async fn dials_without_verification_for_wss() {
    let mut options = HarnessOptions::default();
    options.verification = Verification::Disabled;
    let mut insecure = harness(options, Vec::new());
    insecure.run().await.unwrap();
    assert_eq!(insecure.dials()[0].ssl, RealtimeSsl::Insecure);
}

fn keeps_the_ssl_argument_on_a_ws_url() {
    assert_eq!(
        realtime_ssl("ws://upstream.test/realtime", &Verification::Enabled),
        RealtimeSsl::Shared(Verification::Enabled)
    );
}

async fn closes_after_a_failed_setup_send() {
    let mut options = HarnessOptions::default();
    options.requires_setup = true;
    options.setup = Some("{\"setup\":{}}".to_owned());
    options.backend_send_error = Some("the backend is gone".to_owned());
    let mut setup_failed = harness(options, Vec::new());
    setup_failed.run().await.unwrap();
    assert!(setup_failed.sent().is_empty());
    assert!(setup_failed.closed());
    assert_eq!(setup_failed.close().map(|close| close.0), Some(1011));
}

async fn reports_a_failed_backend_close() {
    let mut options = HarnessOptions::default();
    options.requires_setup = true;
    options.setup = Some("{\"setup\":{}}".to_owned());
    options.backend_close_error = Some("the backend close failed".to_owned());
    let mut close_failed = harness(options, Vec::new());
    close_failed.run().await.unwrap();
    assert_eq!(close_failed.close().map(|close| close.0), Some(1011));
    assert!(
        close_failed
            .texts()
            .iter()
            .any(|text| text.contains("the backend close failed"))
    );
}

async fn replaces_a_send_error_with_the_close_error() {
    let mut options = HarnessOptions::default();
    options.requires_setup = true;
    options.setup = Some("{\"setup\":{}}".to_owned());
    options.backend_send_error = Some("the backend is gone".to_owned());
    options.backend_close_error = Some("the backend close failed".to_owned());
    let mut both_failed = harness(options, Vec::new());
    both_failed.run().await.unwrap();
    let texts = both_failed.texts();
    assert!(
        texts
            .iter()
            .any(|text| text.contains("the backend close failed"))
    );
    assert!(
        !texts
            .iter()
            .any(|text| text.contains("the backend is gone"))
    );
}
