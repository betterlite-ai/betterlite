const NOT_OWNING_TEAM: &str = "You are not a member of the team that owns this submission";
const PLACEHOLDER: &str = "the guardrail-id route";

async fn placeholder() -> &'static str {
    PLACEHOLDER
}

fn identity(user_id: Option<&str>) -> Caller {
    Caller {
        user_id: user_id.map(str::to_owned),
        user_email: None,
        team_id: None,
        role: None,
    }
}

fn assert_unauthorized(answer: &(StatusCode, String)) {
    assert_eq!(answer.0, StatusCode::UNAUTHORIZED);
    let parsed = parse(&answer.1);
    let error = field(&parsed, "error");
    assert_eq!(text(field(error, "message")), "the API key is not valid");
}

#[tokio::test]
async fn get_returns_the_masked_item() {
    let harness = harness();
    let mut stored = row("g1", "alpha", Some("pending_review"), Some("team-a"));
    stored.litellm_params = Some(ColumnJson::Text(
        r#"{"api_key":"secret-key-1234"}"#.to_owned(),
    ));
    harness.rows.insert(stored);
    let answer = send(
        &harness.router,
        get("/guardrails/submissions/g1", Some(admin())),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    let value = parse(&answer.1);
    assert_eq!(text(field(&value, "guardrail_id")), "g1");
    assert_eq!(
        text(field(field(&value, "litellm_params"), "api_key")),
        "se****34"
    );
    assert_eq!(field(&value, "team_guardrail"), &JsonData::Bool(true));
    assert_eq!(text(field(&value, "status")), "pending_review");
}

#[tokio::test]
async fn get_answers_404_for_a_missing_row() {
    let harness = harness();
    let answer = send(
        &harness.router,
        get("/guardrails/submissions/absent", Some(admin())),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::NOT_FOUND,
        "Guardrail submission not found",
    );
}

#[tokio::test]
async fn get_refuses_a_non_admin_outside_the_owning_team() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("active"), Some("team-b")));
    harness
        .rows
        .insert(row("g2", "plain", Some("active"), None));
    harness.teams.allow("alice", &["team-a"]);
    let outside = send(
        &harness.router,
        get("/guardrails/submissions/g1", Some(caller("alice"))),
    )
    .await;
    assert_error(&outside, StatusCode::FORBIDDEN, NOT_OWNING_TEAM);
    let teamless = send(
        &harness.router,
        get("/guardrails/submissions/g2", Some(caller("alice"))),
    )
    .await;
    assert_error(&teamless, StatusCode::FORBIDDEN, NOT_OWNING_TEAM);
}

#[tokio::test]
async fn get_lets_a_member_read_its_teams_row() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("active"), Some("team-a")));
    harness.teams.allow("alice", &["team-a"]);
    let answer = send(
        &harness.router,
        get("/guardrails/submissions/g1", Some(caller("alice"))),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(text(field(&parse(&answer.1), "guardrail_id")), "g1");
}

#[tokio::test]
async fn get_lets_an_admin_read_any_row() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("active"), Some("team-b")));
    let answer = send(
        &harness.router,
        get("/guardrails/submissions/g1", Some(role_admin())),
    )
    .await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
}

#[tokio::test]
async fn get_reports_a_store_error() {
    let harness = harness();
    harness.rows.fail(Failure::Find);
    let (captured, _guard) = capture();
    let answer = send(
        &harness.router,
        get("/guardrails/submissions/g1", Some(admin())),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the guardrail row was not returned by the insert",
    );
    assert!(logged(&captured).contains("Error getting guardrail submission: "));
}

#[tokio::test]
async fn get_answers_500_when_the_team_lookup_fails() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("active"), Some("team-a")));
    harness.teams.fail();
    let answer = send(
        &harness.router,
        get("/guardrails/submissions/g1", Some(caller("alice"))),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys",
    );
}

#[tokio::test]
async fn get_submissions_literal_path_beats_the_id_route() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("active"), Some("team-a")));
    let router = harness
        .router
        .clone()
        .route("/guardrails/{guardrail_id}", get(placeholder));
    let listed = send(&router, get("/guardrails/submissions", Some(admin()))).await;
    assert_eq!(listed.0, StatusCode::OK, "{}", listed.1);
    assert_eq!(ids_of(&listed.1), vec!["g1"]);
    let other = send(&router, get("/guardrails/g1", Some(admin()))).await;
    assert_eq!(other.1, PLACEHOLDER);
}

#[tokio::test]
async fn teams_answer_the_users_teams() {
    let teams = FakeTeams::default();
    teams.allow("alice", &["team-a", "team-b"]);
    let application = state(Some(FakeRows::default()), teams.clone());
    let loaded = user_team_ids(&identity(Some("alice")), &application)
        .await
        .expect("the teams load");
    assert_eq!(loaded, vec!["team-a", "team-b"]);
    assert_eq!(teams.asked(), vec!["alice"]);
}

#[tokio::test]
async fn teams_are_empty_without_a_database() {
    let teams = FakeTeams::default();
    teams.allow("alice", &["team-a"]);
    let application = state(None, teams.clone());
    let loaded = user_team_ids(&identity(Some("alice")), &application)
        .await
        .expect("no teams");
    assert!(loaded.is_empty());
    assert!(teams.asked().is_empty());
}

#[tokio::test]
async fn teams_are_empty_without_a_user_id() {
    let teams = FakeTeams::default();
    let application = state(Some(FakeRows::default()), teams.clone());
    let loaded = user_team_ids(&identity(None), &application)
        .await
        .expect("no teams");
    assert!(loaded.is_empty());
    assert!(teams.asked().is_empty());
}

#[tokio::test]
async fn teams_drop_empty_strings() {
    let teams = FakeTeams::default();
    teams.allow("alice", &["team-a", "", "team-b"]);
    let application = state(Some(FakeRows::default()), teams);
    let loaded = user_team_ids(&identity(Some("alice")), &application)
        .await
        .expect("the teams load");
    assert_eq!(loaded, vec!["team-a", "team-b"]);
}

#[tokio::test]
async fn teams_report_a_lookup_error() {
    let teams = FakeTeams::default();
    teams.fail();
    let application = state(Some(FakeRows::default()), teams);
    let result = user_team_ids(&identity(Some("alice")), &application).await;
    assert!(matches!(result, Err(SubmissionError::Teams { .. })));
}

#[tokio::test]
async fn nodb_register_answers_500() {
    let body = register_request("guard", REGISTER_PARAMS, Some("team-a"));
    let answer = send(
        &bare_router(),
        post("/guardrails/register", Some(admin()), &body),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Prisma client not initialized",
    );
}

#[tokio::test]
async fn nodb_list_answers_500() {
    let answer = send(
        &bare_router(),
        get("/guardrails/submissions", Some(admin())),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Prisma client not initialized",
    );
}

#[tokio::test]
async fn nodb_get_answers_500() {
    let answer = send(
        &bare_router(),
        get("/guardrails/submissions/g1", Some(admin())),
    )
    .await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Prisma client not initialized",
    );
}

#[tokio::test]
async fn nodb_register_without_a_key_is_401() {
    let answer = send(
        &bare_router(),
        post("/guardrails/register", None, "not json"),
    )
    .await;
    assert_unauthorized(&answer);
}

#[tokio::test]
async fn nodb_list_without_a_key_is_401() {
    let answer = send(&bare_router(), get("/guardrails/submissions", None)).await;
    assert_unauthorized(&answer);
}

#[tokio::test]
async fn nodb_get_without_a_key_is_401() {
    let answer = send(&bare_router(), get("/guardrails/submissions/g1", None)).await;
    assert_unauthorized(&answer);
}
