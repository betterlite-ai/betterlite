use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use betterlite_db::budget_permission_rows::JsonData;

use super::super::rows::SessionSpendRow;
use super::support::{
    Events, Failure, Fake, Reply, Scope, admin, call, internal, internal_without_user, json, kinds,
    org_admin, seen, spend_row, state,
};

fn fake(rows: Vec<SessionSpendRow>, row_count: i64) -> Fake {
    Fake {
        events: Events::default(),
        groups: Vec::new(),
        group_count: 0,
        representatives: Vec::new(),
        rows,
        row_count,
        failure: None,
    }
}

fn failing(failure: Failure) -> Fake {
    Fake {
        events: Events::default(),
        groups: Vec::new(),
        group_count: 0,
        representatives: Vec::new(),
        rows: Vec::new(),
        row_count: 0,
        failure: Some(failure),
    }
}

fn scope(teams: Vec<String>, failure: bool) -> Scope {
    Scope {
        events: Events::default(),
        teams,
        failure,
    }
}

fn recorded(events: &Events, kind: &str) -> String {
    seen(events)
        .into_iter()
        .find(|event| event.split_once(' ').is_some_and(|(name, _)| name == kind))
        .unwrap_or_else(|| panic!("no {kind} query"))
}

fn body(reply: &Reply) -> BTreeMap<String, JsonData> {
    let JsonData::Object(fields) = reply.json() else {
        panic!("the body is not an object");
    };
    fields
}

fn row_fields(reply: &Reply) -> BTreeMap<String, JsonData> {
    let fields = body(reply);
    let Some(JsonData::Array(data)) = fields.get("data") else {
        panic!("data is not an array");
    };
    let Some(JsonData::Object(row)) = data.first() else {
        panic!("the page holds no row");
    };
    row.clone()
}

fn issue_kind(issue: &JsonData) -> String {
    let JsonData::Object(fields) = issue else {
        panic!("the issue is not an object");
    };
    let Some(JsonData::String(kind)) = fields.get("type") else {
        panic!("the issue names no type");
    };
    kind.clone()
}

#[tokio::test]
async fn an_admin_reads_the_exact_session() {
    let fake = fake(
        vec![spend_row(
            "r1",
            "s1",
            "2026-01-01T00:00:00",
            Some("bob"),
            None,
        )],
        1,
    );
    let events = Arc::clone(&fake.events);
    let uri = "/spend/logs/session/ui?session_id=s1";
    let reply = call(state(Some(Arc::new(fake)), None), uri, Some(&admin())).await;
    assert_eq!(reply.status, StatusCode::OK);
    let fields = body(&reply);
    assert_eq!(fields.get("total"), Some(&json("1")));
    assert_eq!(fields.get("page"), Some(&json("1")));
    assert_eq!(fields.get("page_size"), Some(&json("50")));
    assert_eq!(fields.get("total_pages"), Some(&json("1")));
    let row = row_fields(&reply);
    assert_eq!(row.get("request_id"), Some(&json("\"r1\"")));
    assert_eq!(kinds(&events), ["row_count", "rows"]);
    let count = recorded(&events, "row_count");
    assert!(count.contains("WHERE session_id = $1"), "{count}");
    let rows_sql = recorded(&events, "rows");
    assert!(
        rows_sql.contains(r#"params=[Text("s1"), Integer(50), Integer(0)]"#),
        "{rows_sql}"
    );
}

#[tokio::test]
async fn an_internal_user_reads_their_own_rows() {
    let fake = fake(
        vec![spend_row(
            "r1",
            "s1",
            "2026-01-01T00:00:00",
            Some("alice"),
            None,
        )],
        1,
    );
    let events = Arc::clone(&fake.events);
    let uri = "/spend/logs/session/ui?session_id=s1";
    let reply = call(
        state(
            Some(Arc::new(fake)),
            Some(Arc::new(scope(Vec::new(), false))),
        ),
        uri,
        Some(&internal("alice")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let count = recorded(&events, "row_count");
    assert!(count.contains(r#""user" = $2"#), "{count}");
    assert!(
        count.contains(r#"params=[Text("s1"), MaybeText(Some("alice"))]"#),
        "{count}"
    );
}

#[tokio::test]
async fn a_permitted_team_widens_the_scope() {
    let fake = fake(Vec::new(), 0);
    let events = Arc::clone(&fake.events);
    let teams = vec![String::from("t1"), String::from("t2")];
    let reply = call(
        state(Some(Arc::new(fake)), Some(Arc::new(scope(teams, false)))),
        "/spend/logs/session/ui?session_id=s1",
        Some(&internal("alice")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let count = recorded(&events, "row_count");
    assert!(
        count.contains(r#"("user" = $2 OR team_id = ANY($3::text[]))"#),
        "{count}"
    );
    assert!(
        count.contains(r#"params=[Text("s1"), MaybeText(Some("alice")), Texts(["t1", "t2"])]"#),
        "{count}"
    );
}

// _get_permitted_team_ids_for_spend_logs_or_empty: a failed lookup still answers the page
#[tokio::test]
async fn a_failed_team_lookup_narrows_to_own_rows() {
    let fake = fake(Vec::new(), 0);
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(
            Some(Arc::new(fake)),
            Some(Arc::new(scope(Vec::new(), true))),
        ),
        "/spend/logs/session/ui?session_id=s1",
        Some(&internal("alice")),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let count = recorded(&events, "row_count");
    assert!(!count.contains("ANY("), "{count}");
    assert!(
        count.contains(r#"params=[Text("s1"), MaybeText(Some("alice"))]"#),
        "{count}"
    );
}

#[tokio::test]
async fn an_org_admin_reads_own_rows_without_a_team_lookup() {
    let fake = fake(Vec::new(), 0);
    let events = Arc::clone(&fake.events);
    let scope = Arc::new(scope(Vec::new(), false));
    let scope_events = Arc::clone(&scope.events);
    let reply = call(
        state(Some(Arc::new(fake)), Some(scope)),
        "/spend/logs/session/ui?session_id=s1",
        Some(&org_admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(seen(&scope_events).is_empty());
    let count = recorded(&events, "row_count");
    assert!(
        count.contains(r#"params=[Text("s1"), MaybeText(Some("olivia"))]"#),
        "{count}"
    );
}

// a null user id stays bound, and the database compares it against nothing
#[tokio::test]
async fn a_caller_without_a_user_id_binds_no_user() {
    let fake = fake(Vec::new(), 0);
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake)), None),
        "/spend/logs/session/ui?session_id=s1",
        Some(&internal_without_user()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let count = recorded(&events, "row_count");
    assert!(
        count.contains(r#"params=[Text("s1"), MaybeText(None)]"#),
        "{count}"
    );
}

#[tokio::test]
async fn an_empty_session_id_is_still_a_value() {
    let fake = fake(Vec::new(), 0);
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake)), None),
        "/spend/logs/session/ui?session_id=",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let count = recorded(&events, "row_count");
    assert!(count.contains(r#"params=[Text("")]"#), "{count}");
}

#[tokio::test]
async fn a_missing_session_id_is_rejected_before_the_database() {
    let reply = call(state(None, None), "/spend/logs/session/ui", Some(&admin())).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        reply.json(),
        json(
            r#"{"detail":[{"type":"missing","loc":["query","session_id"],"msg":"Field required"}]}"#
        )
    );
}

#[tokio::test]
async fn an_unauthenticated_caller_is_turned_away() {
    let fake = fake(Vec::new(), 0);
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake)), None),
        "/spend/logs/session/ui?session_id=s1",
        None,
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert!(seen(&events).is_empty());
}

#[tokio::test]
async fn a_proxy_without_a_database_is_a_500_detail() {
    let reply = call(
        state(None, None),
        "/spend/logs/session/ui?session_id=s1",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(reply.json(), json(r#"{"detail":"Database not connected"}"#));
}

// the count runs first, so its failure leaves no row query behind
#[tokio::test]
async fn a_store_failure_is_a_500_detail_after_the_count() {
    let fake = failing(Failure::Database);
    let events = Arc::clone(&fake.events);
    let reply = call(
        state(Some(Arc::new(fake)), None),
        "/spend/logs/session/ui?session_id=s1",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.json(),
        json(
            r#"{"detail":"No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"}"#
        )
    );
    assert_eq!(kinds(&events), ["row_count"]);
}

// any other read failure is the same 500 detail, carrying the store error's own message
#[tokio::test]
async fn a_row_failure_is_a_500_detail() {
    let fake = failing(Failure::Row);
    let reply = call(
        state(Some(Arc::new(fake)), None),
        "/spend/logs/session/ui?session_id=s1",
        Some(&admin()),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.json(),
        json(r#"{"detail":"the stored row is not a JSON object"}"#)
    );
}

#[tokio::test]
async fn paging_binds_the_limit_and_offset() {
    let fake = fake(
        vec![spend_row(
            "r1",
            "s1",
            "2026-01-01T00:00:00",
            Some("bob"),
            None,
        )],
        25,
    );
    let events = Arc::clone(&fake.events);
    let uri = "/spend/logs/session/ui?session_id=s1&page=3&page_size=10";
    let reply = call(state(Some(Arc::new(fake)), None), uri, Some(&admin())).await;
    assert_eq!(reply.status, StatusCode::OK);
    let fields = body(&reply);
    assert_eq!(fields.get("page"), Some(&json("3")));
    assert_eq!(fields.get("page_size"), Some(&json("10")));
    assert_eq!(fields.get("total"), Some(&json("25")));
    assert_eq!(fields.get("total_pages"), Some(&json("3")));
    let rows_sql = recorded(&events, "rows");
    assert!(
        rows_sql.contains(r#"params=[Text("s1"), Integer(10), Integer(20)]"#),
        "{rows_sql}"
    );
    assert!(rows_sql.contains("LIMIT $2 OFFSET $3"), "{rows_sql}");
}

#[tokio::test]
async fn a_page_size_above_one_hundred_is_rejected() {
    let uri = "/spend/logs/session/ui?session_id=s1&page_size=101";
    let reply = call(state(None, None), uri, Some(&admin())).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let JsonData::Object(fields) = reply.json() else {
        panic!("the body is not an object");
    };
    let Some(JsonData::Array(detail)) = fields.get("detail") else {
        panic!("detail is not an array");
    };
    let Some(JsonData::Object(issue)) = detail.first() else {
        panic!("the issue is not an object");
    };
    assert_eq!(issue.get("type"), Some(&json("\"less_than_equal\"")));
    assert_eq!(
        issue.get("msg"),
        Some(&json("\"Input should be less than or equal to 100\""))
    );
}

#[tokio::test]
async fn every_bad_parameter_is_reported_in_order() {
    let uri = "/spend/logs/session/ui?page=0&page_size=101";
    let reply = call(state(None, None), uri, Some(&admin())).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let JsonData::Object(fields) = reply.json() else {
        panic!("the body is not an object");
    };
    let Some(JsonData::Array(detail)) = fields.get("detail") else {
        panic!("detail is not an array");
    };
    let found: Vec<String> = detail.iter().map(issue_kind).collect();
    assert_eq!(found, ["missing", "greater_than_equal", "less_than_equal"]);
}

#[tokio::test]
async fn metadata_is_decoded_and_tags_stay_text() {
    let fake = fake(
        vec![spend_row(
            "r1",
            "s1",
            "2026-01-01T00:00:00",
            Some("alice"),
            None,
        )],
        1,
    );
    let reply = call(
        state(Some(Arc::new(fake)), None),
        "/spend/logs/session/ui?session_id=s1",
        Some(&admin()),
    )
    .await;
    let row = row_fields(&reply);
    assert_eq!(row.get("metadata"), Some(&json("{\"status\":\"success\"}")));
    assert_eq!(row.get("request_tags"), Some(&json("\"[]\"")));
    assert_eq!(row.get("litellm_call_id"), None);
    assert_eq!(row.get("proxy_server_request"), None);
    assert_eq!(row.get("request_duration_ms"), None);
    assert_eq!(row.get("session_total_count"), None);
    let fields = body(&reply);
    assert!(!fields.contains_key("total_is_capped"));
}
