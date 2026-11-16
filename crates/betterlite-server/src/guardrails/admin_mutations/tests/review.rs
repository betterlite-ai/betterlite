fn review_harness(params: Option<&str>, status: &str) -> Harness {
    let harness = harness();
    harness
        .reviews
        .insert(review_row("g1", "my-guard", params, status));
    harness
}

#[tokio::test]
async fn review_approve_activates_and_starts_the_guardrail() {
    let (captured, _guard) = capture();
    let harness = review_harness(Some(BEDROCK_PARAMS), "pending_review");
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(keys(&parsed).len(), 3);
    assert_eq!(text(field(&parsed, "guardrail_id")), "g1");
    assert_eq!(text(field(&parsed, "status")), "active");
    assert_eq!(text(field(&parsed, "message")), "Guardrail approved");
    assert_eq!(harness.reviews.statuses(), vec!["active".to_owned()]);
    let row = harness.reviews.row("g1").expect("the row stays");
    assert_eq!(row.status, "active");
    assert!(row.reviewed_at.is_some());
    let held = held_guardrail(&harness.runtime, "g1")
        .await
        .expect("the guardrail is held");
    assert_eq!(held.guardrail_name, "my-guard");
    assert_eq!(
        held_source(&harness.runtime, "g1").await,
        Some(GuardrailDefinitionLocation::Db)
    );
    assert!(
        logged(&captured)
            .contains("Approved guardrail my-guard (ID: g1) and initialized in memory")
    );
}

#[tokio::test]
async fn review_approve_reads_params_stored_as_text() {
    let harness = review_harness(
        Some(r#""{\"guardrail\":\"bedrock\",\"mode\":\"pre_call\"}""#),
        "pending_review",
    );
    let (status, _body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(held_guardrail(&harness.runtime, "g1").await.is_some());
}

#[tokio::test]
async fn review_approve_without_params_is_a_500_after_activating() {
    let harness = review_harness(None, "pending_review");
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Guardrail litellm_params is missing or invalid"
    );
    assert_eq!(harness.reviews.statuses(), vec!["active".to_owned()]);
    assert_eq!(harness.reviews.row("g1").unwrap().status, "active");
    assert!(held_guardrail(&harness.runtime, "g1").await.is_none());
}

#[tokio::test]
async fn review_approve_with_empty_params_is_a_500() {
    let harness = review_harness(Some("{}"), "pending_review");
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Guardrail litellm_params is missing or invalid"
    );
    assert_eq!(harness.reviews.statuses(), vec!["active".to_owned()]);
}

#[tokio::test]
async fn review_approve_with_non_object_params_is_a_500() {
    let harness = review_harness(Some("7"), "pending_review");
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Guardrail litellm_params is missing or invalid"
    );
}

#[tokio::test]
async fn review_approve_init_failure_answers_a_200_with_a_warning() {
    let (captured, _guard) = capture();
    let harness = review_harness(
        Some(r#"{"guardrail":"made-up","mode":"pre_call"}"#),
        "pending_review",
    );
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(keys(&parsed).len(), 4);
    assert_eq!(text(field(&parsed, "status")), "active");
    assert_eq!(text(field(&parsed, "message")), "Guardrail approved");
    let warning = text(field(&parsed, "warning"));
    assert!(
        warning.starts_with("Guardrail was marked active but failed to initialize in memory: ")
    );
    assert!(warning.ends_with(" It will be picked up on the next sync cycle."));
    assert_eq!(harness.reviews.row("g1").unwrap().status, "active");
    assert!(held_guardrail(&harness.runtime, "g1").await.is_none());
    assert!(logged(&captured).contains("Failed to initialize approved guardrail g1 in memory: "));
}

#[tokio::test]
async fn review_approve_keeps_a_held_guardrail() {
    let harness = review_harness(Some(BEDROCK_PARAMS), "pending_review");
    {
        let mut handler = harness.runtime.handler().write().await;
        handler
            .initialize_guardrail(
                guardrail_from_row(&stored_row("g1", "config-name", BEDROCK_PARAMS, "active")),
                GuardrailDefinitionLocation::Config,
            )
            .expect("the config guardrail starts");
    }
    let (status, _body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let held = held_guardrail(&harness.runtime, "g1")
        .await
        .expect("the guardrail stays held");
    assert_eq!(held.guardrail_name, "config-name");
    assert_eq!(
        held_source(&harness.runtime, "g1").await,
        Some(GuardrailDefinitionLocation::Db)
    );
}

#[tokio::test]
async fn review_approve_missing_submission_is_a_404() {
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(detail(&body), "Guardrail submission not found");
}

#[tokio::test]
async fn review_approve_not_pending_is_a_400() {
    let harness = review_harness(Some(BEDROCK_PARAMS), "active");
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        detail(&body),
        "Guardrail is not pending review (status=active)"
    );
    assert!(harness.reviews.statuses().is_empty());
}

#[tokio::test]
async fn review_approve_find_error_is_a_500() {
    let (captured, _guard) = capture();
    let harness = harness();
    harness.reviews.fail_find();
    let (status, _body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(logged(&captured).contains("Error approving guardrail submission: "));
}

#[tokio::test]
async fn review_approve_set_error_is_a_500() {
    let (captured, _guard) = capture();
    let harness = review_harness(Some(BEDROCK_PARAMS), "pending_review");
    harness.reviews.fail_set();
    let (status, _body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/approve",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(logged(&captured).contains("Error approving guardrail submission: "));
}

#[tokio::test]
async fn review_reject_marks_the_row_rejected() {
    let harness = review_harness(Some(BEDROCK_PARAMS), "pending_review");
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/reject",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(keys(&parsed).len(), 3);
    assert_eq!(text(field(&parsed, "status")), "rejected");
    assert_eq!(text(field(&parsed, "message")), "Guardrail rejected");
    assert_eq!(harness.reviews.statuses(), vec!["rejected".to_owned()]);
    let row = harness.reviews.row("g1").expect("the row stays");
    assert_eq!(row.status, "rejected");
    assert!(row.reviewed_at.is_some());
    assert!(held_guardrail(&harness.runtime, "g1").await.is_none());
}

#[tokio::test]
async fn review_reject_missing_submission_is_a_404() {
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/reject",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(detail(&body), "Guardrail submission not found");
}

#[tokio::test]
async fn review_reject_not_pending_is_a_400() {
    let harness = review_harness(Some(BEDROCK_PARAMS), "rejected");
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/reject",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        detail(&body),
        "Guardrail is not pending review (status=rejected)"
    );
    assert!(harness.reviews.statuses().is_empty());
}

#[tokio::test]
async fn review_reject_find_error_is_a_500() {
    let (captured, _guard) = capture();
    let harness = harness();
    harness.reviews.fail_find();
    let (status, _body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/reject",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(logged(&captured).contains("Error rejecting guardrail submission: "));
}

#[tokio::test]
async fn review_reject_set_error_is_a_500() {
    let (captured, _guard) = capture();
    let harness = review_harness(Some(BEDROCK_PARAMS), "pending_review");
    harness.reviews.fail_set();
    let (status, _body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails/submissions/g1/reject",
            Some(admin()),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(logged(&captured).contains("Error rejecting guardrail submission: "));
}
