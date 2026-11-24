use axum::http::StatusCode;

use super::support::{
    Answers, Fixture, Reply, StoreMode, admin, internal, json, kinds, metadata_row, query_event,
    row, seen,
};
use crate::spend::log_results::projection::SpendLogListRow;
use crate::spend::sessions::ports::SessionGroupRow;

const UI: &str = "/spend/logs/ui?start_date=2025-01-02%2000:00:00&end_date=2025-01-03%2000:00:00";
const V2: &str = "/spend/logs/v2?start_date=2025-01-02&end_date=2025-01-03";

fn body(reply: &Reply) -> String {
    reply.text()
}

fn session_row() -> SpendLogListRow {
    let mut built = row("r1", Some("alice"), None);
    built.session_id = Some(String::from("s1"));
    built.api_key = Some(String::from("k1"));
    built
}

fn group_row() -> SessionGroupRow {
    SessionGroupRow {
        session_key: String::from("s1"),
        api_key: String::from("k1"),
        last_activity: String::from("2025-01-02 00:00:00"),
    }
}

#[tokio::test]
async fn refuses_a_request_without_an_identity() {
    let (reply, events) = Fixture::default().call(UI, None).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(seen(&events).is_empty());
}

#[tokio::test]
async fn framework_validation_precedes_the_missing_database() {
    let invalid = Fixture::new(StoreMode::Missing, 0);
    let (rejected, _) = invalid
        .call(&format!("{UI}&page=0"), Some(&internal("alice")))
        .await;
    assert_eq!(rejected.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        rejected.json(),
        json(
            r#"{"detail":[{"type":"greater_than_equal","loc":["query","page"],"msg":"Input should be greater than or equal to 1"}]}"#
        )
    );
    let (missing, _) = invalid.call(UI, Some(&internal("alice"))).await;
    assert_eq!(missing.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        missing.json(),
        json(
            r#"{"error":{"message":"Prisma Client is not initialized","type":"internal_error","param":"None","code":"401"}}"#
        )
    );
}

#[tokio::test]
async fn invalid_choices_are_bad_requests_before_any_read() {
    let (reply, events) = Fixture::default()
        .call(&format!("{UI}&sort_by=nope"), Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.json(),
        json(
            r#"{"error":{"message":"Invalid sort_by: nope. Must be one of: endTime, model, request_duration_ms, spend, startTime, total_tokens, ttft_ms","type":"bad_request","param":"sort_by","code":"400"}}"#
        )
    );
    assert!(seen(&events).is_empty());
}

#[tokio::test]
async fn a_window_is_required_except_for_a_ui_id_lookup() {
    let fixture = Fixture::with_rows(vec![row("r1", Some("alice"), None)]);
    let (missing, _) = fixture
        .call("/spend/logs/ui", Some(&internal("alice")))
        .await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);
    assert!(body(&missing).contains("Start date and end date are required"));
    let (looked_up, events) = fixture
        .call("/spend/logs/ui?request_id=r1", Some(&internal("alice")))
        .await;
    assert_eq!(looked_up.status, StatusCode::OK);
    assert_eq!(
        kinds(&events),
        ["precheck", "permitted", "count", "rows", "owner"]
    );
    let (public, _) = fixture.call(V2, Some(&internal("alice"))).await;
    assert_eq!(public.status, StatusCode::OK);
    let (ui_date_only, _) = fixture
        .call(
            "/spend/logs/ui?start_date=2025-01-02&end_date=2025-01-03",
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(ui_date_only.status, StatusCode::BAD_REQUEST);
    assert!(body(&ui_date_only).contains(r#""type":"internal_server_error""#));
}

#[tokio::test]
async fn the_ui_route_enriches_sessions_while_the_public_one_does_not() {
    let fixture = Fixture::with_rows(vec![session_row()]);
    let (ui, ui_events) = fixture.call(UI, Some(&internal("alice"))).await;
    assert_eq!(ui.status, StatusCode::OK);
    assert!(
        seen(&ui_events)
            .iter()
            .any(|event| event == "aggregate [\"s1\"]")
    );
    assert!(body(&ui).contains(r#""session_total_count":1"#));
    let (public, public_events) = fixture.call(V2, Some(&admin())).await;
    assert_eq!(public.status, StatusCode::OK);
    assert!(!kinds(&public_events).iter().any(|kind| kind == "aggregate"));
    assert!(!body(&public).contains("session_total_count"));
}

#[tokio::test]
async fn an_internal_user_reads_their_own_rows_behind_one_predicate() {
    let fixture = Fixture::with_rows(vec![row("r1", Some("alice"), None)]);
    let (reply, events) = fixture.call(UI, Some(&internal("alice"))).await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(kinds(&events), ["permitted", "count", "rows"]);
    let query = query_event(&events, "rows");
    assert!(query.contains(r#"MaybeText(Some("alice"))"#));
    assert!(query.contains(r#""user" = $3"#));
    assert!(query.contains("LIMIT $4 OFFSET $5"));
}

#[tokio::test]
async fn an_explicit_user_filter_stays_an_extra_intersection() {
    let fixture = Fixture::with_rows(Vec::new());
    let (reply, events) = fixture
        .call(&format!("{UI}&user_id=bob"), Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let query = query_event(&events, "rows");
    assert!(query.contains(r#""user" = $3"#));
    assert!(query.contains(r#""user" = $4"#));
    assert!(query.contains(r#"Text("bob")"#));
    assert!(query.contains(r#"MaybeText(Some("alice"))"#));
}

#[tokio::test]
async fn permitted_teams_expand_the_read_to_an_or_clause() {
    let mut fixture = Fixture::with_rows(vec![row("r1", Some("alice"), Some("team-1"))]);
    fixture.permitted = vec![String::from("team-1")];
    let (reply, events) = fixture.call(UI, Some(&internal("alice"))).await;
    assert_eq!(reply.status, StatusCode::OK);
    let query = query_event(&events, "rows");
    assert!(query.contains("OR team_id = ANY($4::text[])"));
    assert!(query.contains(r#"Texts(["team-1"])"#));
}

#[tokio::test]
async fn an_unpermitted_team_filter_is_refused_before_any_read() {
    let fixture = Fixture::default();
    let (reply, events) = fixture
        .call(&format!("{UI}&team_id=t1"), Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.json(),
        json(
            r#"{"error":{"message":"Not authorized to view team spend for team_id=t1","type":"internal_server_error","param":null,"code":"403"}}"#
        )
    );
    assert_eq!(kinds(&events), ["team"]);
}

#[tokio::test]
async fn a_permitted_team_filter_needs_no_user_scope() {
    let mut fixture = Fixture::with_rows(vec![row("r1", Some("bob"), Some("t1"))]);
    fixture.teams = vec![String::from("t1")];
    let (reply, events) = fixture
        .call(&format!("{UI}&team_id=t1"), Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let query = query_event(&events, "rows");
    assert!(query.contains("team_id = $3"));
    assert!(!query.contains(r#""user" = $"#));
    assert_eq!(kinds(&events), ["team", "count", "rows"]);
}

#[tokio::test]
async fn a_foreign_lookup_is_refused_before_reading_the_table() {
    let mut fixture = Fixture::with_rows(vec![row("r1", Some("mallory"), None)]);
    fixture.answers.precheck = false;
    let (reply, events) = fixture
        .call("/spend/logs/ui?request_id=r1", Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.json(),
        json(
            r#"{"error":{"message":"Not authorized to view spend log for request_id=r1","type":"internal_server_error","param":null,"code":"403"}}"#
        )
    );
    assert_eq!(kinds(&events), ["precheck"]);
}

#[tokio::test]
async fn a_row_whose_owner_changed_after_the_precheck_is_refused() {
    let mut fixture = Fixture::with_rows(vec![row("r1", Some("mallory"), None)]);
    fixture.answers.owners = false;
    let (reply, events) = fixture
        .call("/spend/logs/ui?request_id=r1", Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        kinds(&events),
        ["precheck", "permitted", "count", "rows", "owner"]
    );
}

#[tokio::test]
async fn an_authorized_lookup_hydrates_metadata_and_keeps_tag_text() {
    let fixture = Fixture::with_rows(vec![metadata_row(
        "r1",
        r#"{"user_api_key_alias":"alias"}"#,
    )]);
    let (reply, _) = fixture
        .call("/spend/logs/ui?request_id=r1", Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let text = body(&reply);
    assert!(text.contains(r#""metadata":{"user_api_key_alias":"alias"}"#));
    assert!(text.contains(r#""request_tags":"[\"tag\"]""#));
}

#[tokio::test]
async fn the_public_route_never_runs_the_id_checks() {
    let mut fixture = Fixture::with_rows(vec![row("r1", Some("mallory"), None)]);
    fixture.answers = Answers {
        owners: false,
        precheck: false,
    };
    let (reply, events) = fixture
        .call(&format!("{V2}&request_id=r1"), Some(&internal("alice")))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(kinds(&events), ["permitted", "count", "rows"]);
}

#[tokio::test]
async fn the_ui_grouped_start_time_page_delegates_to_the_keyset_pager() {
    let mut fixture = Fixture::with_rows(Vec::new());
    fixture.groups = vec![group_row()];
    fixture.representatives = vec![session_row()];
    let (reply, events) = fixture
        .call(
            &format!("{UI}&group_by_session=true&sort_by=startTime"),
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        kinds(&events),
        ["permitted", "groups", "representatives", "aggregate"]
    );
    assert!(query_event(&events, "groups").contains("DESC"));
    let text = body(&reply);
    assert!(text.contains(r#""next_session_cursor":null"#));
    assert!(text.contains(r#""has_more":false"#));
    assert!(text.contains(r#""total":1"#));
}

#[tokio::test]
async fn a_grouped_sort_other_than_start_time_uses_the_offset_read() {
    let fixture = Fixture::with_rows(vec![row("r1", Some("alice"), None)]);
    let (reply, events) = fixture
        .call(
            &format!("{UI}&group_by_session=true&sort_by=spend"),
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(kinds(&events), ["permitted", "count", "rows"]);
    assert!(query_event(&events, "rows").contains("DISTINCT ON"));
}

#[tokio::test]
async fn a_grouped_public_page_uses_the_offset_read() {
    let mut fixture = Fixture::with_rows(vec![session_row()]);
    fixture.count = 1;
    let (reply, events) = fixture
        .call(
            &format!("{V2}&group_by_session=true&sort_by=startTime"),
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(kinds(&events), ["permitted", "count", "rows"]);
    assert!(query_event(&events, "rows").contains("DISTINCT ON"));
    assert!(query_event(&events, "count").contains("GROUP BY"));
}

#[tokio::test]
async fn a_search_disables_grouping_on_every_route() {
    let mut fixture = Fixture::with_rows(vec![row("r1", Some("alice"), None)]);
    fixture.count = 1;
    let (reply, events) = fixture
        .call(
            &format!("{UI}&group_by_session=true&search=r1"),
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(!query_event(&events, "rows").contains("DISTINCT ON"));
    assert!(!query_event(&events, "count").contains("GROUP BY"));
    assert!(query_event(&events, "rows").contains("litellm_call_id = $1"));
}

#[tokio::test]
async fn a_count_past_the_cap_is_reported_as_the_cap() {
    let mut fixture = Fixture::new(StoreMode::Ready, 10_001);
    let (capped, _) = fixture.call(UI, Some(&internal("alice"))).await;
    let text = body(&capped);
    assert!(text.contains(r#""total":10000"#));
    assert!(text.contains(r#""total_pages":200"#));
    assert!(text.contains(r#""total_is_capped":true"#));
    fixture.count = 10_000;
    let (exact, _) = fixture.call(UI, Some(&internal("alice"))).await;
    assert!(body(&exact).contains(r#""total_is_capped":false"#));
}

#[tokio::test]
async fn the_requested_page_binds_after_every_condition() {
    let fixture = Fixture::new(StoreMode::Ready, 1);
    let (reply, events) = fixture
        .call(
            &format!("{UI}&page=2&page_size=10"),
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let query = query_event(&events, "rows");
    assert!(query.contains("LIMIT $4 OFFSET $5"));
    assert!(query.contains("Integer(10), Integer(10)"));
    assert!(body(&reply).contains(r#""page":2"#));
    assert!(body(&reply).contains(r#""page_size":10"#));
}

#[tokio::test]
async fn a_huge_page_binds_the_largest_offset() {
    let fixture = Fixture::new(StoreMode::Ready, 1);
    let page = i64::MAX;
    let (reply, events) = fixture
        .call(
            &format!("{UI}&page={page}&page_size=1000"),
            Some(&internal("alice")),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let query = query_event(&events, "rows");
    assert!(query.contains("Integer(1000), Integer(9223372036854775807)"));
}

#[tokio::test]
async fn a_store_failure_is_converted_with_its_own_code() {
    let fixture = Fixture::new(StoreMode::Failing, 0);
    let (reply, _) = fixture.call(UI, Some(&internal("alice"))).await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(body(&reply).contains(r#""type":"internal_server_error""#));
    assert!(body(&reply).contains(r#""param":null"#));
    assert!(body(&reply).contains(r#""code":"500""#));
}
