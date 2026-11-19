#[tokio::test]
async fn identity_inserts_a_normalized_login() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":" Alice ","email":"Alice@Example.COM"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: IdentityReply = reply.json();
    assert_eq!(
        body.identity_map.get("alice").map(String::as_str),
        Some("alice@example.com")
    );
    assert!(body.report.is_none());
    let stored = stored_peek(&harness);
    assert_eq!(
        stored.identity_map.get("alice").map(String::as_str),
        Some("alice@example.com")
    );
}

#[tokio::test]
async fn identity_keeps_the_stored_ciphertexts() {
    let harness = Harness::new();
    harness.seed_settings(&ready_stored());
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"octo","email":"octo@example.com"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let stored = stored_peek(&harness);
    assert_eq!(stored.github_token, SEALED_TOKEN);
    assert_eq!(stored.estimator_key, SEALED_ESTIMATOR);
    assert_eq!(stored.repos, vec!["acme/widgets".to_owned()]);
    assert_eq!(stored.estimator_model, "gpt-4o");
}

#[tokio::test]
async fn identity_answers_the_summary_with_the_new_map() {
    let harness = Harness::new();
    harness.seed_settings(&stored_json(
        &["acme/widgets"],
        "gpt-4o",
        60.0,
        &[("alex", "old@example.com")],
    ));
    harness.seed_report(&report_json(&sample_report(fixed_now())));
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"alex","email":"new@example.com"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: IdentityReply = reply.json();
    assert_eq!(
        body.identity_map.get("alex").map(String::as_str),
        Some("new@example.com")
    );
    let summary: SummaryPeek = body.report.expect("summary with the new map");
    assert!(
        summary
            .people
            .iter()
            .any(|person| person.email == "new@example.com")
    );
    assert!(
        !summary
            .people
            .iter()
            .any(|person| person.email == "old@example.com")
    );
}

#[tokio::test]
async fn identity_without_a_stored_report_answers_null_report() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"octo","email":"octo@example.com"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: IdentityReply = reply.json();
    assert!(body.report.is_none());
}

#[tokio::test]
async fn identity_rejects_an_invalid_email() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"octo","email":"not-an-email"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), INVALID_IDENTITY);
    assert!(!recorded(&harness.calls).contains(&"repo.set:roi_calculator_settings".to_owned()));
}

#[tokio::test]
async fn identity_rejects_an_empty_email() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"octo","email":""}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.detail(), INVALID_IDENTITY);
}

#[tokio::test]
async fn identity_removes_the_login_when_email_is_null() {
    let harness = Harness::new();
    harness.seed_settings(&stored_json(
        &["acme/widgets"],
        "gpt-4o",
        60.0,
        &[("alice", "alice@example.com"), ("octo", "octo@example.com")],
    ));
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"Alice","email":null}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: IdentityReply = reply.json();
    assert_eq!(
        body.identity_map.get("octo").map(String::as_str),
        Some("octo@example.com")
    );
    assert!(!body.identity_map.contains_key("alice"));
    let stored = stored_peek(&harness);
    assert!(!stored.identity_map.contains_key("alice"));
    assert!(stored.identity_map.contains_key("octo"));
}

#[tokio::test]
async fn identity_rejects_an_empty_login() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"   ","email":"octo@example.com"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].kind, "value_error");
    assert_eq!(
        issues[0].loc,
        vec!["body".to_owned(), "github_login".to_owned()]
    );
    assert_eq!(issues[0].msg, "Value error, Enter a valid GitHub username.");
    assert!(recorded(&harness.calls).is_empty());
}

#[tokio::test]
async fn identity_rejects_a_missing_login() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(IDENTITY_MAP_PATH, Some(&admin()), r#"{"email":null}"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "missing");
    assert_eq!(
        issues[0].loc,
        vec!["body".to_owned(), "github_login".to_owned()]
    );
    assert_eq!(issues[0].msg, "Field required");
}

#[tokio::test]
async fn identity_rejects_a_missing_email() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"octo"}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "missing");
    assert_eq!(issues[0].loc, vec!["body".to_owned(), "email".to_owned()]);
    assert_eq!(issues[0].msg, "Field required");
}

#[tokio::test]
async fn identity_rejects_a_non_object_body() {
    let harness = Harness::new();
    let reply = call(harness.app(), put(IDENTITY_MAP_PATH, Some(&admin()), "[]")).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "model_attributes_type");
    assert_eq!(issues[0].loc, vec!["body".to_owned()]);
    assert_eq!(
        issues[0].msg,
        "Input should be a valid dictionary or object to extract fields from"
    );
}

#[tokio::test]
async fn identity_rejects_an_empty_body() {
    let harness = Harness::new();
    let reply = call(harness.app(), put(IDENTITY_MAP_PATH, Some(&admin()), "")).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "missing");
    assert_eq!(issues[0].loc, vec!["body".to_owned()]);
    assert_eq!(issues[0].msg, "Field required");
}

#[tokio::test]
async fn identity_rejects_malformed_json() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(IDENTITY_MAP_PATH, Some(&admin()), r#"{"github_login":"#),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "json_invalid");
    assert_eq!(issues[0].loc, vec!["body".to_owned()]);
    assert_eq!(issues[0].msg, "JSON decode error");
}

#[tokio::test]
async fn identity_rejects_a_wrong_email_type() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"octo","email":5}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let issues = reply.issues();
    assert_eq!(issues[0].kind, "value_error");
    assert_eq!(issues[0].loc, vec!["body".to_owned(), "email".to_owned()]);
}

#[tokio::test]
async fn identity_ignores_an_unknown_field() {
    let harness = Harness::new();
    let reply = call(
        harness.app(),
        put(
            IDENTITY_MAP_PATH,
            Some(&admin()),
            r#"{"github_login":"octo","email":null,"extra":1}"#,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: IdentityReply = reply.json();
    assert!(body.identity_map.is_empty());
}
