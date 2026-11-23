#[tokio::test]
async fn headers_keeps_only_the_three_rate_limit_headers() {
    let harness = Harness::new();
    harness.backend.reply(&[
        ("x-ratelimit-remaining-requests", "9"),
        ("x-ratelimit-remaining-tokens", "8"),
        ("x-ms-region", "westeurope"),
        ("x-other-header", "dropped"),
    ]);
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    assert_eq!(
        healthy(outcome),
        HealthCheckResponse {
            remaining_requests: Some("9".to_owned()),
            remaining_tokens: Some("8".to_owned()),
            region: Some("westeurope".to_owned()),
        }
    );
}

#[tokio::test]
async fn headers_partial_reply_leaves_the_missing_headers_unset() {
    let harness = Harness::new();
    harness
        .backend
        .reply(&[("x-ratelimit-remaining-tokens", "8")]);
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    let response = healthy(outcome);
    assert_eq!(response.remaining_requests, None);
    assert_eq!(response.remaining_tokens, Some("8".to_owned()));
    assert_eq!(response.region, None);
}

#[tokio::test]
async fn headers_empty_reply_gives_an_empty_response() {
    let harness = Harness::new();
    let outcome = harness
        .run(params(&[("model", "my-model")]), Some("chat"))
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
}
