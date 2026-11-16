use chrono::Utc;
use uuid::Uuid;

use super::support::cipher::{SALT, cipher};
use super::support::database::{
    AGE_SERVER_SQL, CHILD_COUNT_SQL, ENV_VAR_COUNT_SQL, Harness, OAUTH_COUNT_SQL, SEED_DRAFT_SQL,
    SEED_SERVER_SQL, SERVER_COUNT_SQL, database,
};
use super::support::fake::{FakeStore, Fault, RecordingInvalidation, active_row, draft_row};
use super::support::{json, request};
use crate::mcp::server_writes::draft::{
    create_draft_mcp_server, get_draft_mcp_server, prune_expired_drafts,
};
use crate::mcp::server_writes::{DraftOptions, ServerWriteError};

const APPROVAL_SQL: &str =
    r#"SELECT "approval_status"::text FROM "LiteLLM_MCPServerTable" WHERE "server_id" = $1"#;

async fn seed_draft(harness: &Harness, server_id: &str, age_seconds: f64) {
    let server_name = format!("name-{server_id}");
    harness
        .execute(
            SEED_DRAFT_SQL,
            &[&server_id.to_owned(), &server_name, &"[]".to_owned()],
        )
        .await;
    if age_seconds > 0.0 {
        harness
            .execute(AGE_SERVER_SQL, &[&server_id.to_owned(), &age_seconds])
            .await;
    }
}

async fn seed_real_server(harness: &Harness, server_id: &str, age_seconds: f64) {
    let server_name = format!("name-{server_id}");
    harness
        .execute(
            SEED_SERVER_SQL,
            &[
                &server_id.to_owned(),
                &server_name,
                &None::<String>,
                &None::<String>,
            ],
        )
        .await;
    harness
        .execute(AGE_SERVER_SQL, &[&server_id.to_owned(), &age_seconds])
        .await;
}

fn options(ttl_seconds: i64, server_id: Option<&str>) -> DraftOptions {
    DraftOptions {
        touched_by: "admin".to_owned(),
        ttl_seconds,
        server_id: server_id.map(str::to_owned),
    }
}

#[tokio::test]
async fn create_draft_marks_the_status_and_returns_the_existing_row_untouched() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let invalidation = RecordingInvalidation::new();
    let created = create_draft_mcp_server(
        &store,
        &cipher,
        &request(r#"{"server_name": "flow"}"#),
        &options(300, Some("sess-1")),
        &invalidation,
    )
    .await
    .unwrap();
    assert_eq!(created.server_id, "sess-1");
    assert_eq!(created.server_name.as_deref(), Some("flow"));
    assert_eq!(created.created_by.as_deref(), Some("admin"));
    assert_eq!(
        harness.raw_column(APPROVAL_SQL, "sess-1").await.as_deref(),
        Some("draft")
    );
    let again = create_draft_mcp_server(
        &store,
        &cipher,
        &request(r#"{"server_name": "other", "description": "ignored"}"#),
        &options(300, Some("sess-1")),
        &invalidation,
    )
    .await
    .unwrap();
    assert_eq!(again.server_name.as_deref(), Some("flow"));
    assert_eq!(again.description, None);
    assert_eq!(harness.count(SERVER_COUNT_SQL).await, 1);
    harness.close().await;
}

#[tokio::test]
async fn create_draft_generates_a_uuid_when_no_id_is_given() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let created = create_draft_mcp_server(
        &store,
        &cipher,
        &request(r#"{"server_name": "flow"}"#),
        &DraftOptions::default(),
        &RecordingInvalidation::new(),
    )
    .await
    .unwrap();
    Uuid::parse_str(&created.server_id).expect("the generated id is a uuid");
    assert_eq!(harness.count(SERVER_COUNT_SQL).await, 1);
    harness.close().await;
}

// Every draft write sweeps the expired drafts, with the per-user rows and cached tokens hanging off them.
#[tokio::test]
async fn prune_on_write_removes_expired_drafts_their_children_and_their_tokens() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let invalidation = RecordingInvalidation::new();
    seed_draft(&harness, "old-draft", 600.0).await;
    seed_draft(&harness, "fresh-draft", 0.0).await;
    seed_real_server(&harness, "real", 600.0).await;
    harness.seed_children("old-draft", &["u1"]).await;
    harness.seed_children("real", &["u2"]).await;
    create_draft_mcp_server(
        &store,
        &cipher,
        &request(r#"{"server_name": "new"}"#),
        &options(300, Some("sess")),
        &invalidation,
    )
    .await
    .unwrap();
    assert_eq!(
        harness.server_ids().await,
        ["fresh-draft", "real", "sess"],
        "only the expired draft is swept"
    );
    assert_eq!(harness.count(CHILD_COUNT_SQL).await, 1);
    assert_eq!(harness.count(ENV_VAR_COUNT_SQL).await, 1);
    assert_eq!(harness.count(OAUTH_COUNT_SQL).await, 1);
    assert_eq!(
        invalidation.calls(),
        [("u1".to_owned(), "old-draft".to_owned())],
        "the swept draft's credential users lose their cached tokens"
    );
    harness.close().await;
}

// A zero or negative TTL still keeps one second of window, so the draft just written is never swept.
#[tokio::test]
async fn prune_treats_a_ttl_below_one_as_one_second() {
    let harness = database().await;
    let store = harness.writes();
    seed_draft(&harness, "old", 2.0).await;
    seed_draft(&harness, "fresh", 0.0).await;
    let pruned = prune_expired_drafts(&store, 0, &RecordingInvalidation::new())
        .await
        .unwrap();
    assert_eq!(pruned, 1);
    assert_eq!(harness.server_ids().await, ["fresh"]);
    harness.close().await;
}

#[tokio::test]
async fn prune_skips_a_draft_without_a_timestamp() {
    let row = json(
        r#"{"server_id": "undated", "server_name": "undated", "transport": "sse",
            "approval_status": "draft"}"#,
    );
    let store = FakeStore::new(vec![row]);
    assert_eq!(
        prune_expired_drafts(&store, 0, &RecordingInvalidation::new())
            .await
            .unwrap(),
        0
    );
    assert_eq!(store.rows().len(), 1);
}

// A TTL too large to subtract from now leaves every draft live instead of sweeping it.
#[tokio::test]
async fn prune_keeps_every_draft_when_the_ttl_overflows() {
    let store = FakeStore::new(vec![draft_row("ancient", "2000-01-01T00:00:00Z")]);
    let pruned = prune_expired_drafts(&store, i64::MAX, &RecordingInvalidation::new())
        .await
        .unwrap();
    assert_eq!(pruned, 0);
    assert_eq!(store.rows().len(), 1);
}

// An existing draft comes back like a read does, with its global env vars decrypted.
#[tokio::test]
async fn create_draft_returns_an_existing_draft_with_plaintext_env_vars() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    let invalidation = RecordingInvalidation::new();
    let payload = r#"{"server_name": "flow",
        "env_vars": [{"name": "API_KEY", "value": "global-secret", "scope": "global"}]}"#;
    create_draft_mcp_server(
        &store,
        &cipher,
        &request(payload),
        &options(300, Some("sess-1")),
        &invalidation,
    )
    .await
    .unwrap();
    let existing = create_draft_mcp_server(
        &store,
        &cipher,
        &request(payload),
        &options(300, Some("sess-1")),
        &invalidation,
    )
    .await
    .unwrap();
    let entries = existing.env_vars.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].value, "global-secret");
    harness.close().await;
}

#[tokio::test]
async fn create_draft_returns_an_active_server_untouched() {
    let fresh = active_row("srv-1", &Utc::now().to_rfc3339());
    let store = FakeStore::new(vec![fresh]);
    let found = create_draft_mcp_server(
        &store,
        &cipher(SALT),
        &request(r#"{"server_name": "flow"}"#),
        &options(300, Some("srv-1")),
        &RecordingInvalidation::new(),
    )
    .await
    .unwrap();
    assert_eq!(found.server_id, "srv-1");
    assert_eq!(found.server_name.as_deref(), Some("srv-1"));
    assert_eq!(store.rows().len(), 1, "no draft shadows the real server");
}

// The read and the insert are two statements, so the loser of a create race adopts the winner's draft.
#[tokio::test]
async fn create_draft_adopts_a_draft_written_by_another_worker() {
    let fresh = draft_row("sess-1", &Utc::now().to_rfc3339());
    let mut store = FakeStore::new(vec![fresh]);
    store.fault = Fault::LostCreateRace;
    let adopted = create_draft_mcp_server(
        &store,
        &cipher(SALT),
        &request(r#"{"server_name": "flow"}"#),
        &options(300, Some("sess-1")),
        &RecordingInvalidation::new(),
    )
    .await
    .unwrap();
    assert_eq!(adopted.server_id, "sess-1");
    assert_eq!(store.rows().len(), 1);
}

#[tokio::test]
async fn create_draft_does_not_adopt_an_active_row_from_a_failed_create() {
    let fresh = active_row("sess-1", &Utc::now().to_rfc3339());
    let mut store = FakeStore::new(vec![fresh]);
    store.fault = Fault::LostCreateRace;
    let error = create_draft_mcp_server(
        &store,
        &cipher(SALT),
        &request(r#"{"server_name": "flow"}"#),
        &options(300, Some("sess-1")),
        &RecordingInvalidation::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, ServerWriteError::Table(_)));
}

#[tokio::test]
async fn get_draft_reads_only_live_draft_rows() {
    let harness = database().await;
    let store = harness.writes();
    let cipher = cipher(SALT);
    seed_draft(&harness, "draft", 0.0).await;
    seed_draft(&harness, "expired", 600.0).await;
    seed_real_server(&harness, "real", 0.0).await;
    let found = get_draft_mcp_server(&store, &cipher, "draft", 300)
        .await
        .unwrap();
    assert_eq!(found.unwrap().server_id, "draft");
    assert!(
        get_draft_mcp_server(&store, &cipher, "expired", 300)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        get_draft_mcp_server(&store, &cipher, "real", 300)
            .await
            .unwrap()
            .is_none(),
        "a live server is not a draft"
    );
    assert!(
        get_draft_mcp_server(&store, &cipher, "missing", 300)
            .await
            .unwrap()
            .is_none()
    );
    harness.close().await;
}
