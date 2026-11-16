use super::*;

#[tokio::test]
async fn evaluation_401_evicts_the_obo_token() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![
        token(),
        ScriptedReply::Reply(reply(401, "unauthorized")),
        token(),
        evaluate(r#"{"allowed":true,"defender":{"status":"Evaluated"}}"#),
    ]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail_text(detail, "message"),
        "the Agent 365 evaluation request failed with HTTP 401"
    );
    let recorded = response(record_fields(&data));
    assert_eq!(
        text(recorded.get("reason").unwrap()),
        "HTTP 401: unauthorized"
    );
    let mut second_data = request_data();
    let second = run(&guardrail, &mut second_data).await;
    assert!(second.is_ok());
    let requests = poster.recorded();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].url, TOKEN_ENDPOINT);
    assert_eq!(requests[1].url, EVALUATE_URL);
    assert_eq!(requests[2].url, TOKEN_ENDPOINT);
    assert_eq!(requests[3].url, EVALUATE_URL);
}

#[tokio::test]
async fn evaluation_4xx_is_rejected() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailOpen);
    poster.enqueue(vec![
        token(),
        ScriptedReply::Reply(reply(403, r#"{"error":"forbidden"}"#)),
    ]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 400);
    assert_eq!(
        detail_text(detail, "error"),
        "Agent 365 rejected the tool evaluation request"
    );
    assert_eq!(
        detail_text(detail, "message"),
        "the Agent 365 evaluation request failed with HTTP 403"
    );
    assert!(!detail.contains_key("correlation_id"));
    let fields = record_fields(&data);
    assert_eq!(status_of(fields), "guardrail_intervened");
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Rejected");
    assert_eq!(
        text(recorded.get("reason").unwrap()),
        r#"HTTP 403: {"error":"forbidden"}"#
    );
    assert_eq!(decimal(recorded.get("latency_ms").unwrap()), LATENCY_MILLIS);
}

#[tokio::test]
async fn evaluation_400_uses_the_truncated_body_as_its_message() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    let body = "a".repeat(600);
    poster.enqueue(vec![token(), ScriptedReply::Reply(reply(400, &body))]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 400);
    let truncated = "a".repeat(512);
    assert_eq!(detail_text(detail, "message"), truncated);
    let fields = record_fields(&data);
    let recorded = response(fields);
    assert_eq!(
        text(recorded.get("reason").unwrap()),
        format!("HTTP 400: {truncated}")
    );
}

fn assert_throttled(failure: &EnforcementError, status: u16) {
    let (code, detail) = http_detail(failure);
    assert_eq!(code, 503);
    let reason = format!("the Agent 365 endpoint returned HTTP {status}");
    assert_eq!(
        detail_text(detail, "message"),
        format!(
            "Tool call '{TOOL}' was blocked because {reason}; throttled evaluations block regardless of unreachable_fallback."
        )
    );
    assert_eq!(
        detail_text(detail, "error"),
        "Agent 365 guardrail could not authorize the tool call"
    );
}

#[tokio::test]
async fn evaluation_408_is_throttled() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailOpen);
    poster.enqueue(vec![token(), ScriptedReply::Reply(reply(408, "slow down"))]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    assert_throttled(&failure, 408);
    let fields = record_fields(&data);
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Throttled");
    assert_eq!(
        text(recorded.get("reason").unwrap()),
        "the Agent 365 endpoint returned HTTP 408"
    );
    assert_eq!(status_of(fields), "guardrail_failed_to_respond");
    assert_eq!(decimal(recorded.get("latency_ms").unwrap()), LATENCY_MILLIS);
}

#[tokio::test]
async fn evaluation_429_is_throttled() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailOpen);
    poster.enqueue(vec![token(), ScriptedReply::Reply(reply(429, "too many"))]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    assert_throttled(&failure, 429);
    let recorded = response(record_fields(&data));
    assert_eq!(
        text(recorded.get("reason").unwrap()),
        "the Agent 365 endpoint returned HTTP 429"
    );
}

#[tokio::test]
async fn evaluation_server_error_is_unavailable() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![token(), ScriptedReply::Reply(reply(500, "boom"))]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    assert!(detail_text(detail, "message").contains("the Agent 365 endpoint returned HTTP 500"));
    let fields = record_fields(&data);
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Unavailable");
    assert!(!recorded.contains_key("latency_ms"));
    assert_eq!(duration_of(fields), &GuardrailValue::Null);
}

#[tokio::test]
async fn an_unreachable_evaluation_endpoint_respects_fail_closed() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![token(), ScriptedReply::Unreachable]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    assert!(detail_text(detail, "message").starts_with(&format!(
        "Tool call '{TOOL}' was blocked because the Agent 365 endpoint could not be reached ("
    )));
    let fields = record_fields(&data);
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Unavailable");
    assert!(!recorded.contains_key("latency_ms"));
    assert_eq!(duration_of(fields), &GuardrailValue::Null);
}

#[tokio::test]
async fn an_unreachable_evaluation_endpoint_fails_open() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailOpen);
    poster.enqueue(vec![token(), ScriptedReply::Unreachable]);
    let outcome = run(&guardrail, &mut data).await;
    assert!(outcome.is_ok());
    let recorded = response(record_fields(&data));
    assert_eq!(text(recorded.get("verdict").unwrap()), "Unscanned");
    assert!(
        text(recorded.get("reason").unwrap())
            .starts_with("the Agent 365 endpoint could not be reached (")
    );
}

#[tokio::test]
async fn a_gateway_owned_token_error_follows_unreachable_fallback() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![exchange_failure("invalid_client")]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    let reason = "Entra rejected the gateway's own Agent 365 credentials (invalid_client); check the guardrail's client_id and client_secret";
    assert_eq!(
        detail_text(detail, "message"),
        format!(
            "Tool call '{TOOL}' was blocked because {reason} and unreachable_fallback is 'fail_closed'."
        )
    );
    let recorded = response(record_fields(&data));
    assert_eq!(text(recorded.get("verdict").unwrap()), "Unavailable");
    assert_eq!(text(recorded.get("reason").unwrap()), reason);
}

#[tokio::test]
async fn a_gateway_owned_token_error_fails_open_when_configured() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailOpen);
    poster.enqueue(vec![exchange_failure("invalid_scope")]);
    let outcome = run(&guardrail, &mut data).await;
    assert!(outcome.is_ok());
    let recorded = response(record_fields(&data));
    assert_eq!(text(recorded.get("verdict").unwrap()), "Unscanned");
    assert!(text(recorded.get("reason").unwrap()).contains("(invalid_scope)"));
}

#[tokio::test]
async fn a_caller_owned_token_error_is_a_401() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailOpen);
    poster.enqueue(vec![exchange_failure("invalid_grant")]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 401);
    let reason = "the Entra On-Behalf-Of token exchange was rejected (invalid_grant)";
    assert_eq!(
        detail_text(detail, "message"),
        format!("Tool call '{TOOL}' was blocked because {reason}.")
    );
    assert_eq!(
        detail_text(detail, "error"),
        "Agent 365 guardrail rejected the tool call"
    );
    let recorded = response(record_fields(&data));
    assert_eq!(text(recorded.get("verdict").unwrap()), "Rejected");
    assert_eq!(text(recorded.get("reason").unwrap()), reason);
}

#[tokio::test]
async fn token_endpoint_throttling_blocks_regardless_of_fail_open() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailOpen);
    poster.enqueue(vec![ScriptedReply::Reply(reply(429, "{}"))]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    let reason = "the Entra token endpoint returned HTTP 429";
    assert_eq!(
        detail_text(detail, "message"),
        format!(
            "Tool call '{TOOL}' was blocked because {reason}; throttled evaluations block regardless of unreachable_fallback."
        )
    );
    let fields = record_fields(&data);
    let recorded = response(fields);
    assert_eq!(text(recorded.get("verdict").unwrap()), "Throttled");
    assert!(!recorded.contains_key("latency_ms"));
}

#[tokio::test]
async fn a_malformed_token_reply_is_unavailable() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![ScriptedReply::Reply(reply(200, "not json"))]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    assert!(
        detail_text(detail, "message")
            .contains("the Entra token endpoint returned a non-JSON body")
    );
    let recorded = response(record_fields(&data));
    assert_eq!(text(recorded.get("verdict").unwrap()), "Unavailable");
}

#[tokio::test]
async fn a_token_server_error_is_unavailable() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![ScriptedReply::Reply(reply(500, "boom"))]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    assert!(
        detail_text(detail, "message")
            .starts_with("Tool call 'get_weather' was blocked because the Entra token endpoint could not be reached (")
    );
}

#[tokio::test]
async fn an_unreachable_token_endpoint_respects_fail_closed() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    poster.enqueue(vec![ScriptedReply::Unreachable]);
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let (status, detail) = http_detail(&failure);
    assert_eq!(status, 503);
    assert!(
        detail_text(detail, "message")
            .starts_with("Tool call 'get_weather' was blocked because the Entra token endpoint could not be reached (")
    );
}

#[tokio::test]
async fn an_invalid_tool_name_is_a_bad_request() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    let _previous = data.insert("mcp_tool_name".to_owned(), GuardrailValue::SignedInteger(7));
    let failure = run(&guardrail, &mut data).await.unwrap_err();
    let EnforcementError::Guardrail(GuardrailFailure::Http { status, detail }) = failure else {
        panic!("expected an HTTP failure");
    };
    assert_eq!(status, 400);
    assert_eq!(
        detail,
        "Agent 365 evaluation data field mcp_tool_name must be a string"
    );
    assert!(poster.recorded().is_empty());
}

#[tokio::test]
async fn a_null_tool_name_is_an_empty_name() {
    let Harness {
        guardrail,
        poster,
        mut data,
    } = harness(UnreachableFallback::FailClosed);
    let _previous = data.insert("mcp_tool_name".to_owned(), GuardrailValue::Null);
    poster.enqueue(vec![
        token(),
        evaluate(r#"{"allowed":true,"defender":{"status":"Evaluated"}}"#),
    ]);
    let outcome = run(&guardrail, &mut data).await;
    assert!(outcome.is_ok());
    let request = poster.recorded().remove(1);
    assert_eq!(evaluate_body(&request).tool.name, "");
}
