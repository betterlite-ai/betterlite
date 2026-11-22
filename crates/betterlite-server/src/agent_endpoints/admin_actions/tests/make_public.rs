use axum::http::StatusCode;
use betterlite_core::agents::AgentMakePublicResponse;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;

use super::support::{
    ADMIN, Fixture, MAKE_AGENTS_PUBLIC_PATH, TestDetail, caller, caller_with_user, events, known,
    make_public_path, post, post_json,
};

#[tokio::test]
async fn single_missing_agent_is_404() {
    let fixture = Fixture::new();
    let reply = post(
        fixture.state(),
        &make_public_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(String::from("Agent with ID agent-1 not found"))
    );
}

#[tokio::test]
async fn single_already_public_is_400() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    fixture.groups.seed(&["agent-1"]);
    let reply = post(
        fixture.state(),
        &make_public_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(String::from(
            "Agent with name Agent One already in public agent groups"
        ))
    );
    assert_eq!(fixture.config.saved(), None);
    assert_eq!(
        fixture.groups.current(),
        Some(vec![String::from("agent-1")])
    );
    assert!(events(&fixture.steps).is_empty());
}

#[tokio::test]
async fn single_success_appends_the_agent() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    fixture.groups.seed(&["agent-0"]);
    let reply = post(
        fixture.state(),
        &make_public_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: AgentMakePublicResponse = reply.json();
    assert_eq!(body.message, "Successfully updated public agent groups");
    assert_eq!(
        body.public_agent_groups,
        vec![String::from("agent-0"), String::from("agent-1")]
    );
    assert_eq!(body.updated_by, "user-1");
    assert_eq!(
        fixture.config.saved(),
        Some(body.public_agent_groups.clone())
    );
    assert_eq!(
        fixture.groups.current(),
        Some(body.public_agent_groups.clone())
    );
    assert_eq!(events(&fixture.steps).join(","), "config,memory");
}

#[tokio::test]
async fn single_without_database_is_not_connected() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    let reply = post(
        fixture.state_without_database(),
        &make_public_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(CommonProxyErrors::DbNotConnectedError.as_str().to_owned())
    );
    assert!(events(&fixture.lookups).is_empty());
    assert!(events(&fixture.steps).is_empty());
}

#[tokio::test]
async fn single_checks_the_database_before_the_role() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    let reply = post(
        fixture.state_without_database(),
        &make_public_path("agent-1"),
        &caller("internal_user"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn single_non_admin_is_403() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    let reply = post(
        fixture.state(),
        &make_public_path("agent-1"),
        &caller("internal_user"),
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.detail(),
        TestDetail::Object {
            error: String::from(
                "Only proxy admins can update public model groups. Your role=internal_user"
            ),
        }
    );
    assert!(events(&fixture.lookups).is_empty());
    assert!(events(&fixture.steps).is_empty());
}

#[tokio::test]
async fn single_config_failure_leaves_memory_unchanged() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    fixture.groups.seed(&["agent-0"]);
    fixture.config.fail_with("the config store is down");
    let reply = post(
        fixture.state(),
        &make_public_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(String::from("the config store is down"))
    );
    assert_eq!(fixture.config.saved(), None);
    assert_eq!(
        fixture.groups.current(),
        Some(vec![String::from("agent-0")])
    );
    assert_eq!(events(&fixture.steps).join(","), "config");
}

#[tokio::test]
async fn single_appends_an_agent_found_in_the_database() {
    let fixture = Fixture::new();
    fixture.rows.add(known("agent-1", "Stored Agent"));
    let reply = post(
        fixture.state(),
        &make_public_path("agent-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: AgentMakePublicResponse = reply.json();
    assert_eq!(body.public_agent_groups, vec![String::from("agent-1")]);
    assert_eq!(events(&fixture.lookups).join(","), "registry,row,ids");
}

#[tokio::test]
async fn single_rejects_an_agent_listed_under_a_legacy_id() {
    let fixture = Fixture::new();
    fixture.registry.register(known("stable-1", "Agent One"));
    fixture.registry.alias("stable-1", "legacy-1");
    fixture.groups.seed(&["legacy-1"]);
    let reply = post(
        fixture.state(),
        &make_public_path("stable-1"),
        &caller(ADMIN),
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(String::from(
            "Agent with name Agent One already in public agent groups"
        ))
    );
}

#[tokio::test]
async fn single_without_a_user_id_answers_an_empty_updated_by() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    let reply = post(
        fixture.state(),
        &make_public_path("agent-1"),
        &caller_with_user(ADMIN, None),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: AgentMakePublicResponse = reply.json();
    assert!(body.updated_by.is_empty());
}

#[tokio::test]
async fn bulk_validates_every_agent_before_writing() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    fixture.groups.seed(&["old"]);
    let reply = post_json(
        fixture.state(),
        MAKE_AGENTS_PUBLIC_PATH,
        &caller(ADMIN),
        r#"{"agent_ids":["agent-1","agent-2"]}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(String::from("Agent with ID agent-2 not found"))
    );
    assert_eq!(fixture.config.saved(), None);
    assert_eq!(fixture.groups.current(), Some(vec![String::from("old")]));
    assert!(events(&fixture.steps).is_empty());
}

#[tokio::test]
async fn bulk_success_replaces_the_whole_list() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    fixture.registry.register(known("agent-2", "Agent Two"));
    fixture.groups.seed(&["old"]);
    let reply = post_json(
        fixture.state(),
        MAKE_AGENTS_PUBLIC_PATH,
        &caller(ADMIN),
        r#"{"agent_ids":["agent-1","agent-2"]}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: AgentMakePublicResponse = reply.json();
    assert_eq!(
        body.public_agent_groups,
        vec![String::from("agent-1"), String::from("agent-2")]
    );
    assert_eq!(body.updated_by, "user-1");
    assert_eq!(
        fixture.config.saved(),
        Some(body.public_agent_groups.clone())
    );
    assert_eq!(
        fixture.groups.current(),
        Some(body.public_agent_groups.clone())
    );
    assert_eq!(events(&fixture.steps).join(","), "memory,config");
}

#[tokio::test]
async fn bulk_without_database_is_not_connected() {
    let fixture = Fixture::new();
    let reply = post_json(
        fixture.state_without_database(),
        MAKE_AGENTS_PUBLIC_PATH,
        &caller(ADMIN),
        r#"{"agent_ids":["agent-1"]}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(CommonProxyErrors::DbNotConnectedError.as_str().to_owned())
    );
    assert!(events(&fixture.lookups).is_empty());
    assert!(events(&fixture.steps).is_empty());
}

#[tokio::test]
async fn bulk_non_admin_is_403() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    let reply = post_json(
        fixture.state(),
        MAKE_AGENTS_PUBLIC_PATH,
        &caller("internal_user"),
        r#"{"agent_ids":["agent-1"]}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        reply.detail(),
        TestDetail::Object {
            error: String::from(
                "Only proxy admins can update public model groups. Your role=internal_user"
            ),
        }
    );
    assert!(events(&fixture.lookups).is_empty());
    assert!(events(&fixture.steps).is_empty());
}

#[tokio::test]
async fn bulk_config_failure_keeps_the_replaced_memory() {
    let fixture = Fixture::new();
    fixture.registry.register(known("agent-1", "Agent One"));
    fixture.groups.seed(&["old"]);
    fixture.config.fail_with("the config store is down");
    let reply = post_json(
        fixture.state(),
        MAKE_AGENTS_PUBLIC_PATH,
        &caller(ADMIN),
        r#"{"agent_ids":["agent-1"]}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        reply.detail(),
        TestDetail::Text(String::from("the config store is down"))
    );
    assert_eq!(fixture.config.saved(), None);
    assert_eq!(
        fixture.groups.current(),
        Some(vec![String::from("agent-1")])
    );
    assert_eq!(events(&fixture.steps).join(","), "memory,config");
}

#[tokio::test]
async fn bulk_empty_list_clears_the_public_groups() {
    let fixture = Fixture::new();
    fixture.groups.seed(&["old"]);
    let reply = post_json(
        fixture.state(),
        MAKE_AGENTS_PUBLIC_PATH,
        &caller(ADMIN),
        r#"{"agent_ids":[]}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: AgentMakePublicResponse = reply.json();
    assert!(body.public_agent_groups.is_empty());
    assert_eq!(fixture.config.saved(), Some(Vec::new()));
    assert_eq!(fixture.groups.current(), Some(Vec::new()));
}

#[tokio::test]
async fn bulk_accepts_an_agent_found_in_the_database() {
    let fixture = Fixture::new();
    fixture.rows.add(known("agent-2", "Stored Agent"));
    let reply = post_json(
        fixture.state(),
        MAKE_AGENTS_PUBLIC_PATH,
        &caller(ADMIN),
        r#"{"agent_ids":["agent-2"]}"#,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK);
    let body: AgentMakePublicResponse = reply.json();
    assert_eq!(body.public_agent_groups, vec![String::from("agent-2")]);
}
