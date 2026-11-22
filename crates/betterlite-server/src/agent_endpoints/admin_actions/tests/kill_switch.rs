use axum::http::StatusCode;
use betterlite_config::sections::settings::JsonValue;
use betterlite_core::agents::{AgentKillSwitchMethod, AgentKillSwitchResult};
use secrecy::ExposeSecret as _;
use serde::Deserialize;

use super::support::{
    ADMIN, Fixture, TestDetail, caller, caller_with_user, events, failed_result, key,
    kill_switch_path, known, known_with_switch, master_key_caller, post, post_anonymous,
};
use crate::agent_endpoints::identity_admin::error::Detail;
use crate::agent_endpoints::identity_admin::permission::check_agent_management_permission;
use crate::audit_events::enums::{AuditAction, AuditTable};

const URL: &str = "https://agent.example/stop";

#[derive(Deserialize)]
struct FailureBody {
    detail: AgentKillSwitchResult,
}

#[tokio::test]
async fn missing_agent_is_404() {
    let fixture = Fixture::new();
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(String::from("Agent with ID agent-1 not found"))
    );
    assert!(events(&fixture.steps).is_empty());
}

#[tokio::test]
async fn no_kill_switch_config_is_400() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(String::from(
            "Agent with ID agent-1 has no kill_switch configured"
        ))
    );
    assert!(fixture.trigger.fired().is_empty());
    assert!(events(&fixture.steps).is_empty());
}

#[tokio::test]
async fn success_returns_200_and_writes_the_audit() {
    let fixture = Fixture::new();
    fixture
        .registry
        .register(known_with_switch("agent-1", "Agent One", URL));
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let result: AgentKillSwitchResult = reply.json();
    assert_eq!(result.status_code, Some(200));
    assert_eq!(result.response_body.as_deref(), Some("stopped"));
    let fired = fixture.trigger.fired();
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].0, "agent-1");
    assert_eq!(fired[0].1.url, URL);
    let records = fixture.audit.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "audit-1");
    assert_eq!(records[0].action, AuditAction::KillSwitchFired);
    assert_eq!(records[0].table_name, AuditTable::Agent);
    assert_eq!(records[0].object_id, "agent-1");
    assert_eq!(records[0].changed_by.as_deref(), Some("user-1"));
    let Some(provenance) = records[0].changed_by_api_key.as_ref() else {
        panic!("the audit row names the key that fired the switch");
    };
    assert_ne!(provenance.expose_secret(), "sk-test");
    assert_eq!(
        records[0].updated_at.to_rfc3339(),
        "2026-01-02T03:04:05+00:00"
    );
    let Some(JsonValue::String(values)) = &records[0].updated_values else {
        panic!("the audit row carries the webhook result");
    };
    assert!(values.contains(r#""agent_id":"agent-1""#));
    assert_eq!(events(&fixture.steps).join(","), "fire,audit");
}

#[tokio::test]
async fn webhook_failure_is_502() {
    let fixture = Fixture::new();
    fixture
        .registry
        .register(known_with_switch("agent-1", "Agent One", URL));
    fixture.trigger.answer(failed_result());
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_GATEWAY);
    let body: FailureBody = reply.json();
    assert_eq!(body.detail.agent_id, "agent-1");
    assert_eq!(body.detail.url, URL);
    assert_eq!(body.detail.method, AgentKillSwitchMethod::Post);
    assert_eq!(body.detail.status_code, Some(500));
    assert_eq!(body.detail.response_body.as_deref(), Some("boom"));
    assert!(body.detail.error.is_none());
    assert_eq!(fixture.audit.records().len(), 1);
    assert_eq!(events(&fixture.steps).join(","), "fire,audit");
}

#[tokio::test]
async fn a_db_row_wins_over_the_registry() {
    let fixture = Fixture::new();
    fixture.rows.add(known_with_switch(
        "agent-1",
        "Stored Agent",
        "https://stored.example/stop",
    ));
    fixture.registry.register(known_with_switch(
        "agent-1",
        "Memory Agent",
        "https://memory.example/stop",
    ));
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let fired = fixture.trigger.fired();
    assert_eq!(fired[0].1.url, "https://stored.example/stop");
    assert_eq!(events(&fixture.lookups).join(","), "row");
}

#[tokio::test]
async fn a_db_row_without_a_switch_does_not_fall_back() {
    let fixture = Fixture::new();
    fixture.rows.add(known("agent-1", "Stored Agent"));
    fixture.registry.register(known_with_switch(
        "agent-1",
        "Memory Agent",
        "https://memory.example/stop",
    ));
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert!(fixture.trigger.fired().is_empty());
    assert_eq!(events(&fixture.lookups).join(","), "row");
}

#[tokio::test]
async fn a_db_failure_is_500() {
    let fixture = Fixture::new();
    fixture.rows.fail_with("the agents table is unavailable");
    fixture
        .registry
        .register(known_with_switch("agent-1", "Agent One", URL));
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(String::from("the agents table is unavailable"))
    );
    assert!(fixture.trigger.fired().is_empty());
}

#[tokio::test]
async fn a_non_admin_is_403() {
    let fixture = Fixture::new();
    fixture
        .registry
        .register(known_with_switch("agent-1", "Agent One", URL));
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller("internal_user"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.detail(),
        TestDetail::Object {
            error: String::from(
                "Only proxy admins can create, update, or delete agents. Your role=internal_user"
            ),
        }
    );
    assert!(events(&fixture.lookups).is_empty());
    assert!(events(&fixture.steps).is_empty());
}

#[tokio::test]
async fn an_unauthenticated_request_is_401() {
    let fixture = Fixture::new();
    let reply = post_anonymous(fixture.state(), &kill_switch_path("agent-1")).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_master_key_passes_the_check() {
    let fixture = Fixture::new();
    fixture
        .registry
        .register(known_with_switch("agent-1", "Agent One", URL));
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &master_key_caller(),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        fixture.audit.records()[0].changed_by.as_deref(),
        Some("default_user_id")
    );
}

#[tokio::test]
async fn the_audit_falls_back_to_the_proxy_admin_name() {
    let fixture = Fixture::new();
    fixture
        .registry
        .register(known_with_switch("agent-1", "Agent One", URL));
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller_with_user(ADMIN, None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        fixture.audit.records()[0].changed_by.as_deref(),
        Some("default_user_id")
    );
}

#[tokio::test]
async fn the_check_answers_like_the_b4781_permission() {
    let fixture = Fixture::new();
    let reply = post(
        fixture.state(),
        &kill_switch_path("agent-1"),
        &caller("internal_user"),
    )
    .await;
    let expected = check_agent_management_permission(&key("internal_user")).unwrap_err();
    let expected_detail = match expected.detail {
        Detail::Text(text) => TestDetail::Text(text),
        Detail::Object { error } => TestDetail::Object { error },
    };
    assert_eq!(reply.status.as_u16(), expected.status);
    assert_eq!(reply.detail(), expected_detail);
}
