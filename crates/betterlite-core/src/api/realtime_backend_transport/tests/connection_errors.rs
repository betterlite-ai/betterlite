use super::harness::{HarnessOptions, harness, invalid_status, os};
use crate::api::realtime_backend_transport::error::{ClientCloseError, RealtimeConnectionError};

const HANDSHAKE_401_EVENT: &str = "{\"type\":\"error\",\"error\":{\"type\":\"server_error\",\"message\":\"Upstream realtime handshake rejected with HTTP 401\"}}";

#[rstest::rstest]
#[case::unauthorized(401_u16, 1008_u16)]
#[case::forbidden(403, 1008)]
#[case::rate_limited(429, 1013)]
#[tokio::test]
async fn refusal_closes_with_the_mapped_status(#[case] status: u16, #[case] expected_close: u16) {
    let mut options = HarnessOptions::default();
    options.dials = vec![invalid_status(status)];
    let mut refusal = harness(options, Vec::new());
    refusal.run().await.unwrap();
    assert_eq!(refusal.close().map(|close| close.0), Some(expected_close));
}

#[tokio::test]
async fn refusal_answers_401_with_the_handshake_error() {
    let mut options = HarnessOptions::default();
    options.dials = vec![invalid_status(401)];
    let mut refusal = harness(options, Vec::new());
    refusal.run().await.unwrap();
    assert_eq!(refusal.texts(), vec![HANDSHAKE_401_EVENT.to_owned()]);
    assert_eq!(
        refusal.close(),
        Some((
            1008,
            Some("Upstream realtime handshake rejected with HTTP 401".to_owned())
        ))
    );
}

#[rstest::rstest]
#[case::completed("the response already completed")]
#[case::closing("cannot call websocket.close")]
#[tokio::test]
async fn ignorable_runtime_close_errors_are_swallowed(#[case] message: &str) {
    let mut options = HarnessOptions::default();
    options.dials = vec![os("boom")];
    options.close_error = Some(ClientCloseError::Runtime {
        message: message.to_owned(),
    });
    let mut ignorable = harness(options, Vec::new());
    ignorable.run().await.unwrap();
}

#[tokio::test]
async fn realtime_backend_connection_errors() {
    redacts_the_secret_from_the_error_event().await;
    closes_a_dead_client_with_the_error().await;
    reports_another_runtime_close_error().await;
    reports_a_close_error_of_another_class().await;
}

async fn redacts_the_secret_from_the_error_event() {
    let secret = format!("sk-{}", "a".repeat(48));
    let mut options = HarnessOptions::default();
    options.dials = vec![os(&secret), os(&secret), os(&secret)];
    let mut failed = harness(options, Vec::new());
    failed.run().await.unwrap();
    let texts = failed.texts();
    assert_eq!(texts.len(), 1);
    assert!(!texts[0].contains(&secret));
    assert_eq!(failed.dials().len(), 3);
    let close = failed.close().unwrap();
    assert_eq!(close.0, 1011);
    assert!(!close.1.unwrap().contains(&secret));
}

async fn closes_a_dead_client_with_the_error() {
    let mut options = HarnessOptions::default();
    options.dials = vec![os("boom")];
    options.send_error = Some("the client is gone".to_owned());
    let mut unsendable = harness(options, Vec::new());
    unsendable.run().await.unwrap();
    assert!(unsendable.texts().is_empty());
    assert_eq!(
        unsendable.close(),
        Some((1011, Some("Internal server error: boom".to_owned())))
    );
}

async fn reports_another_runtime_close_error() {
    let mut options = HarnessOptions::default();
    options.dials = vec![os("boom")];
    options.close_error = Some(ClientCloseError::Runtime {
        message: "another runtime failure".to_owned(),
    });
    let mut unexpected = harness(options, Vec::new());
    let error = unexpected.run().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Unexpected error while closing WebSocket: another runtime failure"
    );
}

async fn reports_a_close_error_of_another_class() {
    let mut options = HarnessOptions::default();
    options.dials = vec![os("boom")];
    options.close_error = Some(ClientCloseError::Other {
        message: "the socket is gone".to_owned(),
    });
    let mut other_class = harness(options, Vec::new());
    let error = other_class.run().await.unwrap_err();
    assert!(matches!(error, RealtimeConnectionError::Close(_)));
    assert_eq!(error.to_string(), "the socket is gone");
}
