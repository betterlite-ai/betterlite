#[tokio::test]
async fn create_stores_the_row_and_starts_the_guardrail() {
    let (captured, _guard) = capture();
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request("POST", "/guardrails", Some(admin()), Some(CREATE_JSON)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(text(field(&parsed, "guardrail_name")), "my-guard");
    assert_eq!(
        text(field(&field(&parsed, "litellm_params"), "guardrail")),
        "bedrock"
    );
    assert_eq!(keys(&parsed).len(), 3);
    let guardrail_id = text(field(&parsed, "guardrail_id")).to_owned();
    assert!(!guardrail_id.is_empty());
    assert_eq!(harness.guardrails.created(), 1);
    let stored = harness
        .guardrails
        .row(&guardrail_id)
        .expect("the row is written");
    assert_eq!(stored.guardrail_name, "my-guard");
    assert!(
        held_guardrail(&harness.runtime, &guardrail_id)
            .await
            .is_some()
    );
    assert_eq!(
        held_source(&harness.runtime, &guardrail_id).await,
        Some(GuardrailDefinitionLocation::Db)
    );
    assert!(logged(&captured).contains("Immediate sync: Successfully initialized guardrail"));
}

#[tokio::test]
async fn create_echoes_a_sent_guardrail_info() {
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request("POST", "/guardrails", Some(admin()), Some(CREATE_WITH_INFO)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    let info = field(&parsed, "guardrail_info");
    assert_eq!(text(field(info, "description")), "guard");
    assert_eq!(keys(&parsed).len(), 4);
}

#[tokio::test]
async fn create_echoes_a_null_guardrail_info() {
    let harness = harness();
    let payload = r#"{"guardrail":{"guardrail_name":"g","litellm_params":{"guardrail":"bedrock","mode":"pre_call"},"guardrail_info":null}}"#;
    let (status, body) = send(
        harness.router,
        request("POST", "/guardrails", Some(admin()), Some(payload)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(field(&parse(&body), "guardrail_info"), &JsonData::Null);
}

#[tokio::test]
async fn create_echoes_the_optional_request_columns() {
    let harness = harness();
    let payload = r#"{"guardrail":{"guardrail_id":"client-id","guardrail_name":"my-guard","litellm_params":{"guardrail":"bedrock","mode":"pre_call"},"policy_template":"tpl","created_at":"2026-01-02T03:04:05Z","updated_at":"2026-01-02T03:04:06Z"}}"#;
    let (status, body) = send(
        harness.router,
        request("POST", "/guardrails", Some(admin()), Some(payload)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(keys(&parsed).len(), 6);
    assert_ne!(text(field(&parsed, "guardrail_id")), "client-id");
    assert_eq!(text(field(&parsed, "policy_template")), "tpl");
    assert_eq!(text(field(&parsed, "created_at")), "2026-01-02T03:04:05Z");
    assert_eq!(text(field(&parsed, "updated_at")), "2026-01-02T03:04:06Z");
}

#[tokio::test]
async fn create_rejects_an_unsupported_guardrail() {
    let (captured, _guard) = capture();
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails",
            Some(admin()),
            Some(CREATE_UNSUPPORTED),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        detail(&body),
        "Guardrail configuration error: Unsupported guardrail: made-up"
    );
    assert_eq!(harness.guardrails.created(), 1);
    assert!(harness.guardrails.rows().is_empty());
    assert_eq!(held_count(&harness.runtime).await, 0);
    assert!(!logged(&captured).contains("Successfully initialized"));
}

#[tokio::test]
async fn create_rejects_an_invalid_initializer_config() {
    let harness = harness();
    let payload = r#"{"guardrail":{"guardrail_name":"g","litellm_params":{"guardrail":"bedrock","mode":"pre_call","content_filter_threshold":"high"}}}"#;
    let (status, body) = send(
        harness.router,
        request("POST", "/guardrails", Some(admin()), Some(payload)),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        detail(&body),
        "Guardrail configuration error: the guardrail parameter content_filter_threshold has an invalid value"
    );
    assert!(harness.guardrails.rows().is_empty());
    assert_eq!(held_count(&harness.runtime).await, 0);
}

#[tokio::test]
async fn create_keeps_the_row_when_the_enterprise_package_is_missing() {
    let (captured, _guard) = capture();
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails",
            Some(admin()),
            Some(CREATE_ENTERPRISE),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let guardrail_id = text(field(&parse(&body), "guardrail_id")).to_owned();
    assert!(!guardrail_id.is_empty());
    assert_eq!(harness.guardrails.rows().len(), 1);
    assert_eq!(held_count(&harness.runtime).await, 0);
    assert!(
        logged(&captured).contains("Immediate sync: Failed to initialize guardrail 'my-guard'")
    );
}

#[tokio::test]
async fn create_rollback_failure_is_logged() {
    let (captured, _guard) = capture();
    let harness = harness();
    harness.guardrails.fail(Failure::Delete);
    let (status, _body) = send(
        harness.router,
        request(
            "POST",
            "/guardrails",
            Some(admin()),
            Some(CREATE_UNSUPPORTED),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(logged(&captured).contains("Rollback failed for guardrail '"));
}

#[tokio::test]
async fn create_db_error_is_a_500() {
    let (captured, _guard) = capture();
    let harness = harness();
    harness.guardrails.fail(Failure::Create);
    let (status, body) = send(
        harness.router,
        request("POST", "/guardrails", Some(admin()), Some(CREATE_JSON)),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Error adding guardrail to DB: guardrail row is missing"
    );
    assert!(logged(&captured).contains("Error adding guardrail to db: "));
}

#[tokio::test]
async fn create_accepts_a_platform_admin() {
    let harness = harness();
    let (status, _body) = send(
        harness.router,
        request("POST", "/guardrails", Some(role_admin()), Some(CREATE_JSON)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(harness.guardrails.created(), 1);
}

fn replace_body(name: &str, params: &str) -> String {
    format!(r#"{{"guardrail":{{"guardrail_name":"{name}","litellm_params":{params}}}}}"#)
}

fn stored_params(guardrails: &FakeGuardrails) -> LitellmParams {
    let stored = guardrails
        .row("g1")
        .expect("the row is stored")
        .litellm_params
        .expect("the row has params");
    decode_litellm_params(stored.as_bytes()).expect("the row decodes")
}

fn seed_old_guardrail(harness: &Harness) {
    harness
        .guardrails
        .insert(stored_row("g1", "old-name", BEDROCK_PARAMS, "active"));
    harness
        .reviews
        .insert(review_row("g1", "old-name", None, "active"));
}

fn assert_replaced_body(body: &str) {
    let parsed = parse(body);
    assert_eq!(keys(&parsed).len(), 10);
    assert_eq!(text(field(&parsed, "guardrail_id")), "g1");
    assert_eq!(text(field(&parsed, "guardrail_name")), "new-name");
    assert_eq!(text(field(&parsed, "status")), "active");
    assert_eq!(text(field(&parsed, "submitted_at")), "2026-01-02T03:04:05Z");
}

#[tokio::test]
async fn replace_stores_the_row_and_syncs_memory() {
    let (captured, _guard) = capture();
    let harness = harness();
    seed_old_guardrail(&harness);
    let payload = replace_body(
        "new-name",
        r#"{"guardrail":"bedrock","mode":"pre_call","default_on":false}"#,
    );
    let (status, body) = send(
        harness.router,
        request(
            "PUT",
            "/guardrails/g1",
            Some(admin()),
            Some(payload.as_str()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_replaced_body(&body);
    assert_eq!(
        harness.guardrails.row("g1").unwrap().guardrail_name,
        "new-name"
    );
    let params = stored_params(&harness.guardrails);
    assert_eq!(params.base.fields.default_on, Some(false));
    let held = held_guardrail(&harness.runtime, "g1")
        .await
        .expect("the guardrail is held");
    assert_eq!(held.guardrail_name, "new-name");
    assert_eq!(
        held_source(&harness.runtime, "g1").await,
        Some(GuardrailDefinitionLocation::Db)
    );
    assert!(logged(&captured).contains("Immediate sync: Successfully updated guardrail"));
}

#[tokio::test]
async fn replace_missing_row_is_a_404() {
    let harness = harness();
    let payload = replace_body("new-name", BEDROCK_PARAMS);
    let (status, body) = send(
        harness.router,
        request(
            "PUT",
            "/guardrails/g1",
            Some(admin()),
            Some(payload.as_str()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(detail(&body), "Guardrail with ID g1 not found");
}

#[tokio::test]
async fn replace_rejects_a_configuration_error_and_restores_the_row() {
    let harness = harness();
    seed_old_guardrail(&harness);
    let payload = replace_body("new-name", r#"{"guardrail":"made-up","mode":"pre_call"}"#);
    let (status, body) = send(
        harness.router,
        request(
            "PUT",
            "/guardrails/g1",
            Some(admin()),
            Some(payload.as_str()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        detail(&body),
        "Invalid guardrail configuration, update rejected: Guardrail initialization failed: Unsupported guardrail: made-up"
    );
    assert_eq!(
        harness.guardrails.row("g1").unwrap().guardrail_name,
        "old-name"
    );
    let params = stored_params(&harness.guardrails);
    assert_eq!(params.guardrail, "bedrock");
    assert!(held_guardrail(&harness.runtime, "g1").await.is_none());
}

#[tokio::test]
async fn replace_missing_after_update_is_a_500() {
    let harness = harness();
    harness
        .guardrails
        .insert(stored_row("g1", "old-name", BEDROCK_PARAMS, "active"));
    let payload = replace_body("new-name", BEDROCK_PARAMS);
    let (status, body) = send(
        harness.router,
        request(
            "PUT",
            "/guardrails/g1",
            Some(admin()),
            Some(payload.as_str()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Guardrail with ID g1 not found after the update"
    );
}

#[tokio::test]
async fn replace_find_error_is_a_500() {
    let harness = harness();
    harness.guardrails.fail(Failure::Find);
    let payload = replace_body("new-name", BEDROCK_PARAMS);
    let (status, body) = send(
        harness.router,
        request(
            "PUT",
            "/guardrails/g1",
            Some(admin()),
            Some(payload.as_str()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Error getting guardrail from DB: guardrail row is missing"
    );
}

#[tokio::test]
async fn replace_update_error_is_a_500() {
    let harness = harness();
    harness
        .guardrails
        .insert(stored_row("g1", "old-name", BEDROCK_PARAMS, "active"));
    harness.guardrails.fail(Failure::Update);
    let payload = replace_body("new-name", BEDROCK_PARAMS);
    let (status, body) = send(
        harness.router,
        request(
            "PUT",
            "/guardrails/g1",
            Some(admin()),
            Some(payload.as_str()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Error updating guardrail in DB: guardrail row is missing"
    );
}

#[tokio::test]
async fn patch_merges_only_the_sent_params_keys() {
    let (captured, _guard) = capture();
    let harness = harness();
    harness.guardrails.insert(stored_row(
        "g1",
        "old-name",
        r#"{"guardrail":"bedrock","mode":"pre_call","default_on":true,"guardrailIdentifier":"ff6"}"#,
        "active",
    ));
    harness
        .reviews
        .insert(review_row("g1", "old-name", None, "active"));
    let (status, body) = send(
        harness.router,
        request(
            "PATCH",
            "/guardrails/g1",
            Some(admin()),
            Some(r#"{"litellm_params":{"default_on":false}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let parsed = parse(&body);
    assert_eq!(text(field(&parsed, "guardrail_name")), "old-name");
    let params = stored_params(&harness.guardrails);
    assert_eq!(params.guardrail, "bedrock");
    assert_eq!(params.base.fields.default_on, Some(false));
    assert!(params.base.extra.0.contains_key("guardrailIdentifier"));
    let held = held_guardrail(&harness.runtime, "g1")
        .await
        .expect("the guardrail is held");
    assert_eq!(held.litellm_params.base.fields.default_on, Some(false));
    assert!(logged(&captured).contains("Immediate sync: Successfully updated guardrail"));
}

#[tokio::test]
async fn patch_replaces_info_with_an_empty_object() {
    let harness = harness();
    harness.guardrails.insert(GuardrailRow {
        guardrail_id: "g1".to_owned(),
        guardrail_name: "old-name".to_owned(),
        litellm_params: Some(BEDROCK_PARAMS.to_owned()),
        guardrail_info: Some(r#"{"description":"old"}"#.to_owned()),
        team_id: None,
        status: "active".to_owned(),
        created_at: stamp("2026-01-02T03:04:05"),
        updated_at: stamp("2026-01-02T03:04:05"),
    });
    harness
        .reviews
        .insert(review_row("g1", "old-name", None, "active"));
    let (status, _body) = send(
        harness.router,
        request(
            "PATCH",
            "/guardrails/g1",
            Some(admin()),
            Some(r#"{"guardrail_info":{}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.guardrails.row("g1").unwrap().guardrail_info,
        Some("{}".to_owned())
    );
}

#[tokio::test]
async fn patch_null_info_keeps_the_stored_one() {
    let harness = harness();
    harness.guardrails.insert(GuardrailRow {
        guardrail_id: "g1".to_owned(),
        guardrail_name: "old-name".to_owned(),
        litellm_params: Some(BEDROCK_PARAMS.to_owned()),
        guardrail_info: Some(r#"{"description":"old"}"#.to_owned()),
        team_id: None,
        status: "active".to_owned(),
        created_at: stamp("2026-01-02T03:04:05"),
        updated_at: stamp("2026-01-02T03:04:05"),
    });
    harness
        .reviews
        .insert(review_row("g1", "old-name", None, "active"));
    let (status, _body) = send(
        harness.router,
        request(
            "PATCH",
            "/guardrails/g1",
            Some(admin()),
            Some(r#"{"guardrail_info":null}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.guardrails.row("g1").unwrap().guardrail_info,
        Some(r#"{"description":"old"}"#.to_owned())
    );
}

#[tokio::test]
async fn patch_name_only_keeps_params() {
    let harness = harness();
    seed_old_guardrail(&harness);
    let (status, _body) = send(
        harness.router,
        request(
            "PATCH",
            "/guardrails/g1",
            Some(admin()),
            Some(r#"{"guardrail_name":"renamed"}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        harness.guardrails.row("g1").unwrap().guardrail_name,
        "renamed"
    );
    let params = stored_params(&harness.guardrails);
    assert_eq!(params.guardrail, "bedrock");
}

#[tokio::test]
async fn patch_missing_row_is_a_404() {
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request("PATCH", "/guardrails/g1", Some(admin()), Some("{}")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(detail(&body), "Guardrail with ID g1 not found");
}

#[tokio::test]
async fn patch_rejects_a_configuration_error_and_restores_the_row() {
    let harness = harness();
    seed_old_guardrail(&harness);
    let (status, body) = send(
        harness.router,
        request(
            "PATCH",
            "/guardrails/g1",
            Some(admin()),
            Some(r#"{"litellm_params":{"guardrail":"made-up"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(detail(&body).starts_with("Invalid guardrail configuration, update rejected: "));
    let params = stored_params(&harness.guardrails);
    assert_eq!(params.guardrail, "bedrock");
    assert!(held_guardrail(&harness.runtime, "g1").await.is_none());
}

#[tokio::test]
async fn patch_bad_params_decode_is_a_422() {
    let harness = harness();
    seed_old_guardrail(&harness);
    let (status, body) = send(
        harness.router,
        request(
            "PATCH",
            "/guardrails/g1",
            Some(admin()),
            Some(r#"{"litellm_params":{"on_violation":"block"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        detail(&body),
        "Invalid guardrail configuration, update rejected: on_violation='block' is only supported by guardrail='mcp_security'"
    );
    let params = stored_params(&harness.guardrails);
    assert_eq!(params.guardrail, "bedrock");
    assert_eq!(params.base.fields.on_violation, None);
}

#[tokio::test]
async fn patch_find_error_is_a_500() {
    let (captured, _guard) = capture();
    let harness = harness();
    harness.guardrails.fail(Failure::Find);
    let (status, body) = send(
        harness.router,
        request("PATCH", "/guardrails/g1", Some(admin()), Some("{}")),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Error getting guardrail from DB: guardrail row is missing"
    );
    assert!(logged(&captured).contains("Error updating guardrail: "));
}

#[tokio::test]
async fn patch_update_error_is_a_500() {
    let harness = harness();
    harness
        .guardrails
        .insert(stored_row("g1", "old-name", BEDROCK_PARAMS, "active"));
    harness.guardrails.fail(Failure::Update);
    let (status, body) = send(
        harness.router,
        request(
            "PATCH",
            "/guardrails/g1",
            Some(admin()),
            Some(r#"{"guardrail_name":"renamed"}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Error updating guardrail in DB: guardrail row is missing"
    );
}

#[tokio::test]
async fn patch_missing_after_update_is_a_500() {
    let harness = harness();
    harness
        .guardrails
        .insert(stored_row("g1", "old-name", BEDROCK_PARAMS, "active"));
    let (status, body) = send(
        harness.router,
        request("PATCH", "/guardrails/g1", Some(admin()), Some("{}")),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Guardrail with ID g1 not found after the update"
    );
}

#[tokio::test]
async fn delete_removes_the_row_and_the_memory_entry() {
    let (captured, _guard) = capture();
    let harness = harness();
    harness
        .guardrails
        .insert(stored_row("g1", "old-name", BEDROCK_PARAMS, "active"));
    {
        let mut handler = harness.runtime.handler().write().await;
        handler
            .initialize_guardrail(
                guardrail_from_row(&stored_row("g1", "old-name", BEDROCK_PARAMS, "active")),
                GuardrailDefinitionLocation::Db,
            )
            .expect("the guardrail starts");
    }
    let (status, body) = send(
        harness.router,
        request("DELETE", "/guardrails/g1", Some(admin()), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        text(field(&parse(&body), "message")),
        "Guardrail g1 deleted successfully"
    );
    assert!(harness.guardrails.row("g1").is_none());
    assert!(held_guardrail(&harness.runtime, "g1").await.is_none());
    assert!(
        logged(&captured).contains(
            "Immediate sync: Successfully removed guardrail 'Unknown' (ID: g1) from memory"
        )
    );
}

#[tokio::test]
async fn delete_missing_row_is_a_404() {
    let harness = harness();
    let (status, body) = send(
        harness.router,
        request("DELETE", "/guardrails/missing", Some(admin()), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(detail(&body), "Guardrail with ID missing not found");
}

#[tokio::test]
async fn delete_removes_a_pending_review_row() {
    let harness = harness();
    harness.guardrails.insert(stored_row(
        "g1",
        "old-name",
        BEDROCK_PARAMS,
        "pending_review",
    ));
    let (status, _body) = send(
        harness.router,
        request("DELETE", "/guardrails/g1", Some(admin()), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(harness.guardrails.row("g1").is_none());
}

#[tokio::test]
async fn delete_db_error_is_a_500() {
    let harness = harness();
    harness
        .guardrails
        .insert(stored_row("g1", "old-name", BEDROCK_PARAMS, "active"));
    harness.guardrails.fail(Failure::Delete);
    let (status, body) = send(
        harness.router,
        request("DELETE", "/guardrails/g1", Some(admin()), None),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Error deleting guardrail from DB: guardrail row is missing"
    );
}

#[tokio::test]
async fn delete_find_error_is_a_500() {
    let harness = harness();
    harness.guardrails.fail(Failure::Find);
    let (status, body) = send(
        harness.router,
        request("DELETE", "/guardrails/g1", Some(admin()), None),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        detail(&body),
        "Error getting guardrail from DB: guardrail row is missing"
    );
}
