fn submissions_of(payload: &str) -> Vec<JsonData> {
    let value = parse(payload);
    array(field(&value, "submissions")).to_vec()
}

fn summary_of(payload: &str) -> JsonData {
    field(&parse(payload), "summary").clone()
}

fn ids_of(payload: &str) -> Vec<String> {
    submissions_of(payload)
        .iter()
        .map(|entry| text(field(entry, "guardrail_id")).to_owned())
        .collect()
}

fn count(payload: &str, name: &str) -> u64 {
    let summary = summary_of(payload);
    let JsonData::Number(value) = field(&summary, name) else {
        panic!("{name} is not a number");
    };
    value.as_u64().expect("a count")
}

async fn list(harness: &Harness, auth: AuthContext, query: &str) -> (StatusCode, String) {
    let uri = format!("/guardrails/submissions{query}");
    send(&harness.router, get(&uri, Some(auth))).await
}

#[tokio::test]
async fn list_gives_admins_every_team_row() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("pending_review"), Some("team-a")));
    harness
        .rows
        .insert(row("g2", "beta", Some("active"), Some("team-b")));
    harness
        .rows
        .insert(row("g3", "plain", Some("active"), None));
    let answer = list(&harness, admin(), "").await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(ids_of(&answer.1), vec!["g1", "g2"]);
    assert_eq!(count(&answer.1, "total"), 2);
    assert_eq!(count(&answer.1, "pending_review"), 1);
    assert_eq!(count(&answer.1, "active"), 1);
    assert_eq!(count(&answer.1, "rejected"), 0);
}

#[tokio::test]
async fn list_gives_a_non_admin_only_its_teams() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("pending_review"), Some("team-a")));
    harness
        .rows
        .insert(row("g2", "beta", Some("active"), Some("team-b")));
    harness.teams.allow("alice", &["team-a"]);
    let answer = list(&harness, caller("alice"), "").await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(ids_of(&answer.1), vec!["g1"]);
    assert_eq!(count(&answer.1, "total"), 1);
    assert_eq!(harness.rows.team_queries(), 1);
}

#[tokio::test]
async fn list_refuses_a_team_filter_outside_the_callers_teams() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("active"), Some("team-b")));
    harness.teams.allow("alice", &["team-a"]);
    let outside = list(&harness, caller("alice"), "?team_id=team-b").await;
    assert_error(
        &outside,
        StatusCode::FORBIDDEN,
        "You are not a member of team 'team-b'",
    );
    let empty = list(&harness, caller("alice"), "?team_id=").await;
    assert_error(
        &empty,
        StatusCode::FORBIDDEN,
        "You are not a member of team ''",
    );
    assert_eq!(harness.rows.team_queries(), 0);
}

#[tokio::test]
async fn list_answers_empty_for_a_non_admin_without_teams() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("active"), Some("team-a")));
    let answer = list(&harness, caller("alice"), "").await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert!(submissions_of(&answer.1).is_empty());
    assert_eq!(count(&answer.1, "total"), 0);
    assert_eq!(count(&answer.1, "pending_review"), 0);
    assert_eq!(count(&answer.1, "active"), 0);
    assert_eq!(count(&answer.1, "rejected"), 0);
    assert_eq!(harness.rows.team_queries(), 0);
}

#[tokio::test]
async fn list_counts_the_summary_before_the_filters() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("pending_review"), Some("team-a")));
    harness
        .rows
        .insert(row("g2", "beta", Some("active"), Some("team-a")));
    harness
        .rows
        .insert(row("g3", "gamma", Some("rejected"), Some("team-a")));
    harness
        .rows
        .insert(row("g4", "delta", None, Some("team-a")));
    let answer = list(&harness, admin(), "?status=active").await;
    assert_eq!(answer.0, StatusCode::OK, "{}", answer.1);
    assert_eq!(ids_of(&answer.1), vec!["g2"]);
    assert_eq!(count(&answer.1, "total"), 4);
    assert_eq!(count(&answer.1, "pending_review"), 1);
    assert_eq!(count(&answer.1, "active"), 2);
    assert_eq!(count(&answer.1, "rejected"), 1);
}

#[tokio::test]
async fn list_filters_by_the_raw_status() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", None, Some("team-a")));
    harness
        .rows
        .insert(row("g2", "beta", Some("active"), Some("team-a")));
    let answer = list(&harness, admin(), "?status=active").await;
    assert_eq!(ids_of(&answer.1), vec!["g2"]);
    assert_eq!(count(&answer.1, "active"), 2);
}

#[tokio::test]
async fn list_filters_by_team() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha", Some("active"), Some("team-a")));
    harness
        .rows
        .insert(row("g2", "beta", Some("active"), Some("team-b")));
    let admin_view = list(&harness, admin(), "?team_id=team-b").await;
    assert_eq!(ids_of(&admin_view.1), vec!["g2"]);
    assert_eq!(count(&admin_view.1, "total"), 2);
    harness.teams.allow("alice", &["team-a", "team-b"]);
    let member_view = list(&harness, caller("alice"), "?team_id=team-b").await;
    assert_eq!(ids_of(&member_view.1), vec!["g2"]);
}

#[tokio::test]
async fn list_searches_the_name() {
    let harness = harness();
    harness
        .rows
        .insert(row("g1", "alpha-guard", Some("active"), Some("team-a")));
    harness
        .rows
        .insert(row("g2", "beta-guard", Some("active"), Some("team-a")));
    let answer = list(&harness, admin(), "?search=ALPHA").await;
    assert_eq!(ids_of(&answer.1), vec!["g1"]);
    assert_eq!(count(&answer.1, "total"), 2);
}

#[tokio::test]
async fn list_searches_a_description() {
    let harness = harness();
    let info: JsonObject = serde_json::from_str(r#"{"description":"Checks the Beta words"}"#)
        .expect("the fixture is an object");
    let mut described = row("g1", "alpha", Some("active"), Some("team-a"));
    described.guardrail_info = Some(ColumnJson::Object(info));
    harness.rows.insert(described);
    harness
        .rows
        .insert(row("g2", "gamma", Some("active"), Some("team-a")));
    let answer = list(&harness, admin(), "?search=beta").await;
    assert_eq!(ids_of(&answer.1), vec!["g1"]);
    let checks = list(&harness, admin(), "?search=checks").await;
    assert_eq!(ids_of(&checks.1), vec!["g1"]);
}

#[tokio::test]
async fn list_searches_the_whole_info_text() {
    let harness = harness();
    let mut stored = row("g1", "alpha", Some("active"), Some("team-a"));
    stored.guardrail_info = Some(ColumnJson::Text(
        r#"{"description":"none","submitted_by_email":"OPS@example.com"}"#.to_owned(),
    ));
    harness.rows.insert(stored);
    let answer = list(&harness, admin(), "?search=ops").await;
    assert_eq!(ids_of(&answer.1), vec!["g1"]);
}

#[tokio::test]
async fn list_reports_a_store_error() {
    let harness = harness();
    harness.rows.fail(Failure::TeamRows);
    let (captured, _guard) = capture();
    let answer = list(&harness, admin(), "").await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the guardrail row was not returned by the insert",
    );
    assert!(logged(&captured).contains("Error listing guardrail submissions: "));
}

#[tokio::test]
async fn list_reports_a_team_lookup_error() {
    let harness = harness();
    harness.teams.fail();
    let answer = list(&harness, caller("alice"), "").await;
    assert_error(
        &answer,
        StatusCode::INTERNAL_SERVER_ERROR,
        "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys",
    );
}
