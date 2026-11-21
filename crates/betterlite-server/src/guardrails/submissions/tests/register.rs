const TEAM_REQUIRED: &str =
    "team_id is required. Provide it in the request body or use a team-scoped API key.";

async fn register(harness: &Harness, auth: AuthContext, body: &str) -> (StatusCode, String) {
    send(
        &harness.router,
        post("/guardrails/register", Some(auth), body),
    )
    .await
}

fn created_info(harness: &Harness) -> JsonData {
    let created = harness.rows.created();
    let stored = created.last().expect("one record");
    parse(&stored.guardrail_info)
}

fn assert_stored_submission(harness: &Harness) {
    let created = harness.rows.created();
    let stored = created.first().expect("one record");
    assert_eq!(stored.status, "pending_review");
    assert_eq!(stored.team_id, "team-a");
    assert_eq!(stored.submitted_at, fixed_now());
    assert_eq!(stored.created_at, fixed_now());
    assert_eq!(stored.updated_at, fixed_now());
    let params = parse(&stored.litellm_params);
    assert_eq!(text(field(&params, "guardrail")), "generic_guardrail_api");
}

fn user_key(team: Option<&str>, user_id: &str) -> AuthContext {
    let team_id = team.unwrap_or_default();
    keyed(&[
        ("user_role", "internal_user"),
        ("team_id", team_id),
        ("user_id", user_id),
    ])
}

#[tokio::test]
async fn register_stores_a_pending_submission() {
    let harness = harness();
    let auth = keyed(&[
        ("user_role", "internal_user"),
        ("team_id", "team-a"),
        ("user_id", "alice"),
        ("user_email", "alice@example.com"),
    ]);
    let body = register_request("guard", REGISTER_PARAMS, Some("team-a"));
    let answer = register(&harness, auth, &body).await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    let value = parse(&answer.1);
    assert_eq!(text(field(&value, "guardrail_id")), "created-1");
    assert_eq!(text(field(&value, "guardrail_name")), "guard");
    assert_eq!(text(field(&value, "status")), "pending_review");
    let submitted = DateTime::parse_from_rfc3339(text(field(&value, "submitted_at")))
        .expect("a timestamp")
        .with_timezone(&Utc);
    assert_eq!(submitted, fixed_now());
    assert_stored_submission(&harness);
    assert!(harness.teams.asked().is_empty());
    let info = created_info(&harness);
    assert_eq!(text(field(&info, "submitted_by_user_id")), "alice");
    assert_eq!(
        text(field(&info, "submitted_by_email")),
        "alice@example.com"
    );
    assert_eq!(field(&info, "team_guardrail"), &JsonData::Bool(true));
}

#[tokio::test]
async fn register_uses_the_key_team_when_the_body_has_none() {
    let harness = harness();
    let answer = register(
        &harness,
        user_key(Some("team-a"), "alice"),
        &register_request("guard", REGISTER_PARAMS, None),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(harness.rows.created()[0].team_id, "team-a");
    assert!(harness.teams.asked().is_empty());
}

#[tokio::test]
async fn register_falls_back_when_the_body_team_is_empty() {
    let harness = harness();
    let answer = register(
        &harness,
        user_key(Some("team-a"), "alice"),
        &register_request("guard", REGISTER_PARAMS, Some("")),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(harness.rows.created()[0].team_id, "team-a");
    assert!(harness.teams.asked().is_empty());
}

#[tokio::test]
async fn register_requires_a_team() {
    let harness = harness();
    let answer = register(
        &harness,
        user_key(None, "alice"),
        &register_request("guard", REGISTER_PARAMS, None),
    )
    .await;
    assert_error(&answer, StatusCode::BAD_REQUEST, TEAM_REQUIRED);
    assert!(harness.rows.created().is_empty());
}

#[tokio::test]
async fn register_lets_a_member_register_for_another_team() {
    let harness = harness();
    harness.teams.allow("alice", &["team-b"]);
    let answer = register(
        &harness,
        user_key(Some("team-a"), "alice"),
        &register_request("guard", REGISTER_PARAMS, Some("team-b")),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(harness.rows.created()[0].team_id, "team-b");
    assert_eq!(harness.teams.asked(), vec!["alice"]);
}

#[tokio::test]
async fn register_refuses_a_non_member() {
    let harness = harness();
    harness.teams.allow("alice", &["team-a"]);
    let answer = register(
        &harness,
        user_key(Some("team-a"), "alice"),
        &register_request("guard", REGISTER_PARAMS, Some("team-b")),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::FORBIDDEN,
        "You are not a member of team 'team-b'",
    );
    assert!(harness.rows.created().is_empty());
}

#[tokio::test]
async fn register_skips_the_lookup_for_the_keys_own_team() {
    let harness = harness();
    harness.teams.fail();
    let answer = register(
        &harness,
        user_key(Some("team-a"), "alice"),
        &register_request("guard", REGISTER_PARAMS, Some("team-a")),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(harness.rows.created()[0].team_id, "team-a");
}

#[tokio::test]
async fn register_lets_a_proxy_admin_register_any_team() {
    let harness = harness();
    harness.teams.fail();
    let auth = keyed(&[("user_role", "proxy_admin"), ("user_id", "root")]);
    let answer = register(
        &harness,
        auth,
        &register_request("guard", REGISTER_PARAMS, Some("team-b")),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(harness.rows.created()[0].team_id, "team-b");
}

#[tokio::test]
async fn register_does_not_let_a_view_only_admin_register_for_another_team() {
    let harness = harness();
    harness.teams.allow("alice", &["team-a"]);
    let auth = keyed(&[
        ("user_role", "proxy_admin_viewer"),
        ("team_id", "team-a"),
        ("user_id", "alice"),
    ]);
    let answer = register(
        &harness,
        auth,
        &register_request("guard", REGISTER_PARAMS, Some("team-b")),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::FORBIDDEN,
        "You are not a member of team 'team-b'",
    );
}

#[tokio::test]
async fn register_rejects_a_duplicate_name() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "guard", Some("rejected"), Some("team-b")));
    harness
        .rows
        .insert(row("g2", "plain", Some("active"), None));
    let held = register(
        &harness,
        admin(),
        &register_request("guard", REGISTER_PARAMS, Some("team-a")),
    )
    .await;
    assert_error(
        &held,
        StatusCode::BAD_REQUEST,
        "Guardrail with name 'guard' already exists",
    );
    let teamless = register(
        &harness,
        admin(),
        &register_request("plain", REGISTER_PARAMS, Some("team-a")),
    )
    .await;
    assert_error(
        &teamless,
        StatusCode::BAD_REQUEST,
        "Guardrail with name 'plain' already exists",
    );
    assert!(harness.rows.created().is_empty());
}

#[tokio::test]
async fn register_reports_a_uniqueness_store_error() {
    let harness = harness();
    harness.rows.fail(Failure::Name);
    let (captured, _guard) = capture();
    let answer = register(
        &harness,
        admin(),
        &register_request("guard", REGISTER_PARAMS, Some("team-a")),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the guardrail row was not returned by the insert",
    );
    assert!(logged(&captured).contains("Error checking guardrail name uniqueness: "));
}

#[tokio::test]
async fn register_reports_a_create_store_error() {
    let harness = harness();
    harness.rows.fail(Failure::Create);
    let (captured, _guard) = capture();
    let answer = register(
        &harness,
        admin(),
        &register_request("guard", REGISTER_PARAMS, Some("team-a")),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the guardrail row was not returned by the insert",
    );
    assert!(logged(&captured).contains("Error registering guardrail: "));
}

#[tokio::test]
async fn register_reports_a_team_lookup_error() {
    let harness = harness();
    harness.teams.fail();
    let answer = register(
        &harness,
        user_key(Some("team-a"), "alice"),
        &register_request("guard", REGISTER_PARAMS, Some("team-b")),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys",
    );
}

#[tokio::test]
async fn register_uses_the_user_rows_email_first() {
    let harness = harness();
    let auth = keyed(&[
        ("user_role", "internal_user"),
        ("team_id", "team-a"),
        ("user_email", "key@example.com"),
    ]);
    let auth = with_user(auth, "alice", Some("row@example.com"));
    let answer = register(
        &harness,
        auth,
        &register_request("guard", REGISTER_PARAMS, None),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    let info = created_info(&harness);
    assert_eq!(text(field(&info, "submitted_by_email")), "row@example.com");
    assert_eq!(text(field(&info, "submitted_by_user_id")), "alice");
}

#[tokio::test]
async fn register_keeps_a_missing_email_null() {
    let harness = harness();
    let answer = register(
        &harness,
        admin(),
        &register_request("guard", REGISTER_PARAMS, Some("team-a")),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(
        field(&created_info(&harness), "submitted_by_email"),
        &JsonData::Null
    );
    harness.teams.allow("alice", &["team-a"]);
    let auth = keyed(&[
        ("user_role", "internal_user"),
        ("user_email", "key@example.com"),
    ]);
    let auth = with_user(auth, "alice", None);
    let answer = register(
        &harness,
        auth,
        &register_request("second", REGISTER_PARAMS, Some("team-a")),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(
        field(&created_info(&harness), "submitted_by_email"),
        &JsonData::Null
    );
}

#[tokio::test]
async fn register_merges_the_body_guardrail_info() {
    let harness = harness();
    let body = format!(
        r#"{{"guardrail_name":"guard","litellm_params":{REGISTER_PARAMS},"team_id":"team-a","guardrail_info":{{"description":"custom","team_guardrail":false,"submitted_by_user_id":"spoof"}}}}"#
    );
    let answer = register(&harness, admin(), &body).await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    let info = created_info(&harness);
    assert_eq!(text(field(&info, "description")), "custom");
    assert_eq!(field(&info, "team_guardrail"), &JsonData::Bool(true));
    assert_eq!(
        text(field(&info, "submitted_by_user_id")),
        "default_user_id"
    );
}
