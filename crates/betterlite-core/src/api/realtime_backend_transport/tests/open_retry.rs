use std::time::Duration;

use betterlite_config::tls::Verification;
use betterlite_models::decisions::Json;

use super::harness::{
    DialOutcome, HarnessOptions, SourceOutcome, harness, invalid_status, invalid_status_code, os,
    parse, websocket,
};
use crate::api::realtime_backend_transport::dial::{RealtimeSsl, realtime_ssl};
use crate::api::realtime_backend_transport::error::RealtimeConnectionError;
use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::realtime_session_state::state::SessionFlag;

#[tokio::test(start_paused = true)]
async fn realtime_backend_open_retry() {
    retries_after_two_hung_handshakes().await;
    reports_the_last_dial_error().await;
    refuses_with_the_status_close().await;
    reports_the_invalid_status_code().await;
    uses_a_provider_owned_backend().await;
    fails_config_before_dialing().await;
    passes_the_validated_headers_to_the_dialer().await;
    closes_when_the_provider_backend_cannot_open().await;
    dials_without_verification_for_wss().await;
    keeps_the_ssl_argument_on_a_ws_url();
    closes_after_a_failed_setup_send().await;
    reports_a_failed_backend_close().await;
    replaces_a_send_error_with_the_close_error().await;
}

async fn retries_after_two_hung_handshakes() {
    let options = HarnessOptions::default()
        .with_dials(vec![
            DialOutcome::Hang,
            DialOutcome::Hang,
            DialOutcome::Connect,
        ])
        .with_requires_setup(SessionFlag(true))
        .with_setup(Some("{\"setup\":{}}".to_owned()));
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
    assert_eq!(
        dials[1].at.duration_since(dials[0].at),
        Duration::from_secs(8)
    );
    assert_eq!(
        dials[2].at.duration_since(dials[1].at),
        Duration::from_secs(8)
    );
    assert_eq!(retry.sent().len(), 1);
    assert!(retry.closed());
    assert_eq!(retry.close(), None);
    let outcome = retry.observation.lock().unwrap().source_outcome.clone();
    assert!(matches!(
        outcome,
        SourceOutcome::Closed {
            code: Some(1000),
            ..
        }
    ));
}

async fn reports_the_last_dial_error() {
    let options =
        HarnessOptions::default().with_dials(vec![os("first"), websocket("second"), os("third")]);
    let mut failing = harness(options, Vec::new());
    failing.run().await.unwrap();
    assert_eq!(failing.dials().len(), 3);
    assert_eq!(
        failing.close(),
        Some((1011, Some("Internal server error: third".to_owned())))
    );
}

async fn refuses_with_the_status_close() {
    let options = HarnessOptions::default().with_dials(vec![invalid_status(401)]);
    let mut rejected = harness(options, Vec::new());
    rejected.run().await.unwrap();
    assert_eq!(rejected.dials().len(), 1);
    assert_eq!(rejected.close().map(|close| close.0), Some(1008));
}

async fn reports_the_invalid_status_code() {
    let options = HarnessOptions::default().with_dials(vec![invalid_status_code(
        403,
        "server rejected WebSocket connection: HTTP 403",
    )]);
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
    let options = HarnessOptions::default().with_own_backend(SessionFlag(true));
    let mut provider_owned = harness(options, Vec::new());
    provider_owned.run().await.unwrap();
    assert!(provider_owned.dials().is_empty());
    assert!(provider_owned.closed());
}

async fn fails_config_before_dialing() {
    let options =
        HarnessOptions::default().with_validate_error(Some("the api key is missing".to_owned()));
    let mut invalid_headers = harness(options, Vec::new());
    let error = invalid_headers.run().await.unwrap_err();
    assert!(matches!(error, RealtimeConnectionError::Config(_)));
    assert!(invalid_headers.dials().is_empty());
    assert!(invalid_headers.texts().is_empty());
    assert!(invalid_headers.close().is_none());
}

async fn passes_the_validated_headers_to_the_dialer() {
    let mut validated = harness(HarnessOptions::default(), Vec::new());
    validated.run().await.unwrap();
    let dials = validated.dials();
    assert_eq!(dials.len(), 1);
    let expected = HeaderMap::from([
        (
            "Authorization".to_owned(),
            Json::Text("Bearer test".to_owned()),
        ),
        (
            "x-provider-validated".to_owned(),
            Json::Text("yes".to_owned()),
        ),
    ]);
    assert_eq!(dials[0].headers, expected);
    assert_eq!(validated.open_backend_headers(), vec![expected]);
}

async fn closes_when_the_provider_backend_cannot_open() {
    let secret = format!("sk-{}", "a".repeat(48));
    let options = HarnessOptions::default()
        .with_open_backend_error(Some(format!("{secret} the provider backend is gone")));
    let mut provider_failed = harness(options, Vec::new());
    provider_failed.run().await.unwrap();
    assert!(provider_failed.dials().is_empty());
    assert_eq!(provider_failed.open_backend_headers().len(), 1);
    let texts = provider_failed.texts();
    assert_eq!(texts.len(), 1);
    assert!(!texts[0].contains(&secret));
    let Json::Object(event) = parse(&texts[0]) else {
        panic!("the server error event is not a JSON object");
    };
    assert_eq!(event.get("type"), Some(&Json::Text("error".to_owned())));
    assert!(matches!(
        event.get("error"),
        Some(Json::Object(error))
            if error.get("type") == Some(&Json::Text("server_error".to_owned()))
    ));
    let close = provider_failed.close().unwrap();
    assert_eq!(close.0, 1011);
    let reason = close.1.unwrap();
    assert!(!reason.contains(&secret));
    assert!(reason.starts_with("Internal server error: "));
    assert!(reason.contains("the provider backend is gone"));
}

async fn dials_without_verification_for_wss() {
    let options = HarnessOptions::default().with_verification(Verification::Disabled);
    let mut insecure = harness(options, Vec::new());
    insecure.run().await.unwrap();
    assert_eq!(insecure.dials()[0].ssl, RealtimeSsl::Insecure);
}

fn keeps_the_ssl_argument_on_a_ws_url() {
    assert_eq!(
        realtime_ssl("ws://upstream.test/realtime", &Verification::Enabled),
        RealtimeSsl::Shared(Verification::Enabled)
    );
    assert_eq!(
        realtime_ssl("ws://upstream.test/realtime", &Verification::Disabled),
        RealtimeSsl::Disabled
    );
    assert_eq!(
        realtime_ssl("wss://upstream.test/realtime", &Verification::Disabled),
        RealtimeSsl::Insecure
    );
}

async fn closes_after_a_failed_setup_send() {
    let options = HarnessOptions::default()
        .with_requires_setup(SessionFlag(true))
        .with_setup(Some("{\"setup\":{}}".to_owned()))
        .with_backend_send_error(Some("the backend is gone".to_owned()));
    let mut setup_failed = harness(options, Vec::new());
    setup_failed.run().await.unwrap();
    assert!(setup_failed.sent().is_empty());
    assert!(setup_failed.closed());
    assert_eq!(setup_failed.close().map(|close| close.0), Some(1011));
}

async fn reports_a_failed_backend_close() {
    let options = HarnessOptions::default()
        .with_requires_setup(SessionFlag(true))
        .with_setup(Some("{\"setup\":{}}".to_owned()))
        .with_backend_close_error(Some("the backend close failed".to_owned()));
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
    let options = HarnessOptions::default()
        .with_requires_setup(SessionFlag(true))
        .with_setup(Some("{\"setup\":{}}".to_owned()))
        .with_backend_send_error(Some("the backend is gone".to_owned()))
        .with_backend_close_error(Some("the backend close failed".to_owned()));
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
