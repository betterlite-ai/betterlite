use super::harness::{HarnessOptions, harness, invalid_status, os};
use crate::api::realtime_backend_transport::error::{ClientCloseError, RealtimeConnectionError};

#[tokio::test]
async fn realtime_backend_connection_errors() {
    for (status, expected_close) in [(401_u16, 1008_u16), (403, 1008), (429, 1013)] {
        let mut options = HarnessOptions::default();
        options.dials = vec![invalid_status(status)];
        let mut refusal = harness(options, Vec::new());
        refusal.run().await.unwrap();
        assert_eq!(refusal.close().map(|close| close.0), Some(expected_close));
        if status == 401 {
            assert_eq!(
                refusal.texts(),
                vec![format!(
                    "{{\"type\":\"error\",\"error\":{{\"type\":\"server_error\",\"message\":\"Upstream realtime handshake rejected with HTTP 401\"}}}}"
                )]
            );
            assert_eq!(
                refusal.close(),
                Some((
                    1008,
                    Some("Upstream realtime handshake rejected with HTTP 401".to_owned())
                ))
            );
        }
    }

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

    for message in [
        "the response already completed",
        "cannot call websocket.close",
    ] {
        let mut options = HarnessOptions::default();
        options.dials = vec![os("boom")];
        options.close_error = Some(ClientCloseError::Runtime {
            message: message.to_owned(),
        });
        let mut ignorable = harness(options, Vec::new());
        ignorable.run().await.unwrap();
    }

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
