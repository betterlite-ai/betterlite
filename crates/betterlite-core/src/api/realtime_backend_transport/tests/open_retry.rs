use std::time::Duration;

use betterlite_config::tls::Verification;

use super::harness::{
    DialOutcome, HarnessOptions, harness, invalid_status, invalid_status_code, os, websocket,
};
use crate::api::realtime_backend_transport::dial::RealtimeSsl;
use crate::api::realtime_backend_transport::error::RealtimeConnectionError;

#[tokio::test(start_paused = true)]
async fn realtime_backend_open_retry() {
    let mut options = HarnessOptions::default();
    options.dials = vec![DialOutcome::Hang, DialOutcome::Hang, DialOutcome::Connect];
    options.requires_setup = true;
    options.setup = Some("{\"setup\":{}}".to_owned());
    let mut retry = harness(options, Vec::new());
    retry.run().await.unwrap();
    let dials = retry.dials();
    assert_eq!(dials.len(), 3);
    for dial in &dials {
        assert_eq!(dial.url, "wss://upstream.test/realtime");
        assert_eq!(dial.max_size, 1024);
        assert_eq!(dial.open_timeout, Duration::from_secs(8));
        assert_eq!(dial.ssl, RealtimeSsl::Shared(Verification::Enabled));
    }
    assert_eq!(dials[1].at - dials[0].at, Duration::from_secs(8));
    assert_eq!(dials[2].at - dials[1].at, Duration::from_secs(8));
    assert_eq!(retry.sent().len(), 1);
    assert!(retry.closed());
    assert_eq!(
        retry.close(),
        Some((
            1000,
            Some("upstream websocket closed with code 1000".to_owned())
        ))
    );

    let mut options = HarnessOptions::default();
    options.dials = vec![os("first"), websocket("second"), os("third")];
    let mut failing = harness(options, Vec::new());
    let error = failing.run().await.unwrap_err();
    assert_eq!(error.to_string(), "third");
    assert_eq!(failing.dials().len(), 3);

    let mut options = HarnessOptions::default();
    options.dials = vec![invalid_status(401)];
    let mut rejected = harness(options, Vec::new());
    rejected.run().await.unwrap();
    assert_eq!(rejected.dials().len(), 1);
    assert_eq!(rejected.close().map(|close| close.0), Some(1008));

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

    let mut options = HarnessOptions::default();
    options.own_backend = true;
    let mut provider_owned = harness(options, Vec::new());
    provider_owned.run().await.unwrap();
    assert!(provider_owned.dials().is_empty());
    assert!(provider_owned.closed());

    let mut options = HarnessOptions::default();
    options.validate_error = Some("the api key is missing".to_owned());
    let mut invalid_headers = harness(options, Vec::new());
    let error = invalid_headers.run().await.unwrap_err();
    assert!(matches!(error, RealtimeConnectionError::Config(_)));
    assert!(invalid_headers.dials().is_empty());
    assert!(invalid_headers.texts().is_empty());
    assert!(invalid_headers.close().is_none());
}
