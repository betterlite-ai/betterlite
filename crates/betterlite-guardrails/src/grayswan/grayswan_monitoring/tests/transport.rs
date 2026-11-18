#[tokio::test]
async fn call_grayswan_api_posts_to_the_monitor_path() {
    let (capture, base, _server) =
        capture_server(answer(AxumStatus::OK, r#"{"violation": 0.2}"#)).await;
    let settings = configured(&format!("{base}/"), OnFlaggedAction::Monitor, true);
    let response = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect("response");
    assert!((response.violation - 0.2).abs() < f64::EPSILON);
    let recorded = capture.0.lock().unwrap();
    assert_eq!(recorded.method.as_deref(), Some("POST"));
    assert_eq!(recorded.path.as_deref(), Some("/cygnal/monitor"));
}

#[tokio::test]
async fn call_grayswan_api_sends_the_auth_headers() {
    let (capture, base, _server) = capture_server(answer(AxumStatus::OK, "{}")).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect("response");
    let recorded = capture.0.lock().unwrap();
    assert_eq!(recorded.authorization.as_deref(), Some("Bearer secret"));
    assert_eq!(recorded.api_key.as_deref(), Some("secret"));
    assert_eq!(recorded.content_type.as_deref(), Some("application/json"));
}

#[tokio::test]
async fn call_grayswan_api_sends_the_payload_as_json() {
    let (capture, base, _server) = capture_server(answer(AxumStatus::OK, "{}")).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect("response");
    let body = captured_payload(&capture);
    assert_eq!(body.messages, vec![message("user", "hi")]);
    assert!(body.tools.is_none());
    assert!(body.litellm_metadata.is_none());
}

#[tokio::test]
async fn call_grayswan_api_rejects_a_client_error_status() {
    let (_capture, base, _server) = capture_server(answer(AxumStatus::NOT_FOUND, "{}")).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    let error = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect_err("a client error status is an error");
    assert_eq!(error.status_code, Some(404));
    assert_eq!(error.http_status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn call_grayswan_api_rejects_a_server_error_status() {
    let (_capture, base, _server) =
        capture_server(answer(AxumStatus::INTERNAL_SERVER_ERROR, "{}")).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    let error = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect_err("a server error status is an error");
    assert_eq!(error.status_code, Some(500));
    assert_eq!(error.http_status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn call_grayswan_api_rejects_a_redirect_response() {
    let (_capture, base, _server) =
        capture_server(answer(AxumStatus::FOUND, r#"{"violation": 0.3}"#)).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    let error = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect_err("a 3xx response is an error like every non-2xx");
    assert!(matches!(
        error.cause,
        Some(GraySwanFailure::HttpStatus { .. })
    ));
    assert_eq!(error.status_code, Some(302));
    assert_eq!(error.http_status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn call_grayswan_api_logs_the_monitor_response_at_debug() {
    let (_capture, base, _server) =
        capture_server(answer(AxumStatus::OK, r#"{"violation": 0.2}"#)).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    let captured = super::Captured::default();
    let guard = tracing::subscriber::set_default(super::subscriber(
        captured.clone(),
        tracing::Level::DEBUG,
    ));
    call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect("response");
    drop(guard);
    let output = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(output.contains("Gray Swan Guardrail: monitor response"));
    assert!(output.contains("violation: 0.2"));
}

#[tokio::test]
async fn call_grayswan_api_rejects_an_invalid_json_body() {
    let (_capture, base, _server) = capture_server(answer(AxumStatus::OK, "not json")).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    let error = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect_err("invalid json is an error");
    assert!(matches!(
        error.cause,
        Some(GraySwanFailure::ResponseJson { .. })
    ));
    assert_eq!(error.status_code, None);
    assert_eq!(error.http_status(), StatusCode::INTERNAL_SERVER_ERROR);
}

const SPEC_MAX_RESPONSE_BYTES: usize = 0x0010_0000;

#[tokio::test]
async fn call_grayswan_api_accepts_a_body_at_the_size_limit() {
    assert_eq!(MAX_RESPONSE_BYTES, SPEC_MAX_RESPONSE_BYTES);
    let body = format!("{{}}{}", " ".repeat(SPEC_MAX_RESPONSE_BYTES - 2));
    assert_eq!(body.len(), SPEC_MAX_RESPONSE_BYTES);
    let (_capture, base, _server) = capture_server(answer(AxumStatus::OK, &body)).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    let response = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect("a body at the limit is accepted");
    assert!((response.violation - 0.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn call_grayswan_api_rejects_a_body_over_the_size_limit() {
    let body = " ".repeat(SPEC_MAX_RESPONSE_BYTES + 1);
    let (_capture, base, _server) = capture_server(answer(AxumStatus::OK, &body)).await;
    let settings = configured(&base, OnFlaggedAction::Monitor, true);
    let error = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect_err("an oversized body is an error");
    let Some(GraySwanFailure::ResponseTooLarge { limit }) = error.cause else {
        panic!("expected ResponseTooLarge");
    };
    assert_eq!(limit, SPEC_MAX_RESPONSE_BYTES);
    assert_eq!(error.status_code, None);
}

#[tokio::test]
async fn call_grayswan_api_reads_a_streamed_body() {
    let response = call_streaming(vec![Ok(Bytes::from("{}")), Ok(Bytes::from(" "))])
        .await
        .expect("a streamed body is read");
    assert!((response.violation - 0.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn call_grayswan_api_reports_a_stream_failure() {
    let error = call_streaming(vec![Ok(Bytes::from("{}")), Err("broken".to_owned())])
        .await
        .expect_err("a broken stream is an error");
    assert!(matches!(error.cause, Some(GraySwanFailure::Request { .. })));
}

#[tokio::test]
async fn call_grayswan_api_times_out() {
    let (_capture, base, _server) = capture_server(Answer {
        status: AxumStatus::OK,
        body: "{}".to_owned(),
        delay: Some(Duration::from_millis(200)),
    })
    .await;
    let mut settings = configured(&base, OnFlaggedAction::Monitor, true);
    settings.guardrail_timeout = Duration::from_millis(20);
    let error = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect_err("a slow server times out");
    assert!(matches!(error.cause, Some(GraySwanFailure::Timeout { .. })));
    assert_eq!(error.status_code, Some(408));
    assert_eq!(error.http_status(), StatusCode::REQUEST_TIMEOUT);
}

#[tokio::test]
async fn call_grayswan_api_reports_a_connection_failure() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let settings = configured(&format!("http://{addr}"), OnFlaggedAction::Monitor, true);
    let error = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect_err("an unreachable server is an error");
    assert!(matches!(error.cause, Some(GraySwanFailure::Request { .. })));
    assert_eq!(error.status_code, None);
}

#[tokio::test]
async fn call_grayswan_api_rejects_an_invalid_key_before_sending() {
    let mut settings = configured("http://127.0.0.1:1", OnFlaggedAction::Monitor, true);
    settings.api_key = SecretString::from("bad\u{0}key");
    let error = call_grayswan_api(&reqwest::Client::default(), &settings, &payload())
        .await
        .expect_err("an invalid header is an error");
    assert!(matches!(
        error.cause,
        Some(GraySwanFailure::InvalidHeader { .. })
    ));
}
