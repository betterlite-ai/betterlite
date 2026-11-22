use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::JsonData;

use super::support::{
    RacingStore, admin, alice, caller, execute, finish, forbidden, postgres, seeded, user,
    visibility,
};
use crate::spend::visibility::error::VisibilityError;
use crate::spend::visibility::owners::OwnerPair;
use crate::spend::visibility::postgres::PostgresVisibility;
use crate::spend::visibility::store::VisibilityStore as _;

const ALICE_TEAMS: [&str; 3] = ["team-c", "team-b", "team-a"];

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn pair(owner_user: Option<&str>, owner_team: Option<&str>) -> OwnerPair {
    OwnerPair {
        user: owner_user.map(str::to_owned),
        team_id: owner_team.map(str::to_owned),
    }
}

#[tokio::test]
async fn owner_read_returns_every_distinct_pair_once() {
    let seeded = seeded().await;
    let store = PostgresVisibility {
        store: seeded.store.clone(),
    };
    let owners = store.owners("call-shared").await.unwrap();
    assert_eq!(owners.len(), 2);
    let distinct: BTreeSet<OwnerPair> = owners.into_iter().collect();
    let expected = BTreeSet::from([
        pair(Some("alice"), None),
        pair(Some("mallory"), Some("team-x")),
    ]);
    assert_eq!(distinct, expected);
    assert_eq!(
        store.owners("req-team").await.unwrap(),
        vec![pair(Some("bob"), Some("team-a"))]
    );
    assert!(store.owners("missing").await.unwrap().is_empty());
    finish(seeded).await;
}

#[tokio::test]
async fn colliding_call_ids_never_lock_the_owner_out() {
    let seeded = seeded().await;
    let checks = postgres(&seeded, vec![user("alice", &ALICE_TEAMS)]);
    checks
        .assert_can_view_request_id(&alice(), "call-shared")
        .await
        .unwrap();
    let resolved = checks
        .resolve_payload_row(&alice(), "call-shared")
        .await
        .unwrap();
    let row = resolved.row.unwrap();
    assert_eq!(resolved.storage_key, "req-own");
    assert_eq!(row.request_id, "req-own");
    assert_eq!(row.messages, Some(json(r#"[{"role":"user"}]"#)));
    assert_eq!(row.response, Some(json(r#"{"id":"resp-1"}"#)));
    assert_eq!(row.proxy_server_request, Some(json(r#"{"model":"m"}"#)));
    assert_eq!(
        row.metadata,
        Some(json(r#"{"user_api_key_user_id":"alice"}"#))
    );
    assert_eq!(row.user.as_deref(), Some("alice"));
    assert_eq!(row.team_id, None);
    finish(seeded).await;
}

#[tokio::test]
async fn foreign_and_missing_ids_fail_the_same_403() {
    let seeded = seeded().await;
    let checks = postgres(&seeded, vec![user("alice", &ALICE_TEAMS)]);
    let foreign = checks
        .assert_can_view_request_id(&alice(), "call-foreign")
        .await;
    assert_eq!(forbidden(foreign), "call-foreign");
    let missing = checks.assert_can_view_request_id(&alice(), "missing").await;
    assert_eq!(forbidden(missing), "missing");
    let unpermitted = checks
        .assert_can_view_request_id(&alice(), "call-no-permission")
        .await;
    assert_eq!(forbidden(unpermitted), "call-no-permission");
    // team "" names alice its admin, yet an empty owner team is never looked up
    let blank = checks
        .assert_can_view_request_id(&alice(), "req-blank-team")
        .await;
    assert_eq!(forbidden(blank), "req-blank-team");
    finish(seeded).await;
}

#[tokio::test]
async fn team_rows_pass_for_admins_and_spend_permission_holders() {
    let seeded = seeded().await;
    let checks = postgres(&seeded, vec![user("alice", &ALICE_TEAMS)]);
    checks
        .assert_can_view_request_id(&alice(), "call-team")
        .await
        .unwrap();
    checks
        .assert_can_view_request_id(&alice(), "call-admin-team")
        .await
        .unwrap();
    // direct ownership needs no internal-user role
    let team_key = caller(Some("alice"), Some(UserRole::Team));
    checks
        .assert_can_view_request_id(&team_key, "req-own")
        .await
        .unwrap();
    let resolved = checks
        .resolve_payload_row(&alice(), "call-admin-team")
        .await
        .unwrap();
    assert_eq!(resolved.storage_key, "req-admin-team");
    finish(seeded).await;
}

#[tokio::test]
async fn permitted_teams_follow_stored_membership_in_row_order() {
    let seeded = seeded().await;
    let users = vec![
        user("alice", &["team-c", "team-b", "team-a", "team-gone"]),
        user("loner", &[]),
    ];
    let checks = postgres(&seeded, users);
    let permitted = checks.permitted_team_ids(&alice()).await.unwrap();
    assert_eq!(permitted, vec!["team-a".to_owned(), "team-c".to_owned()]);
    let loner = caller(Some("loner"), Some(UserRole::InternalUser));
    assert!(checks.permitted_team_ids(&loner).await.unwrap().is_empty());
    let nobody = caller(None, Some(UserRole::InternalUser));
    assert!(checks.permitted_team_ids(&nobody).await.unwrap().is_empty());
    finish(seeded).await;
}

#[tokio::test]
async fn payload_read_prefers_the_exact_request_id() {
    let seeded = seeded().await;
    execute(
        &seeded.pool,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, litellm_call_id, "user") VALUES ('req-call-dup', 'dup', 'alice'), ('dup', NULL, 'alice')"#,
    )
    .await;
    let checks = postgres(&seeded, vec![user("alice", &ALICE_TEAMS)]);
    let owned = checks.resolve_payload_row(&alice(), "dup").await.unwrap();
    assert_eq!(owned.storage_key, "dup");
    let unscoped = checks.resolve_payload_row(&admin(), "dup").await.unwrap();
    assert_eq!(unscoped.storage_key, "dup");
    finish(seeded).await;
}

#[tokio::test]
async fn a_foreign_row_inserted_after_the_precheck_is_not_served() {
    let seeded = seeded().await;
    let checks = postgres(&seeded, vec![user("alice", &ALICE_TEAMS)]);
    checks
        .assert_can_view_request_id(&alice(), "call-shared")
        .await
        .unwrap();
    execute(
        &seeded.pool,
        r#"INSERT INTO "LiteLLM_SpendLogs" (request_id, "user", team_id) VALUES ('call-shared', 'mallory', 'team-x')"#,
    )
    .await;
    let owned = checks
        .resolve_payload_row(&alice(), "call-shared")
        .await
        .unwrap();
    assert_eq!(owned.storage_key, "req-own");
    let unscoped = checks
        .resolve_payload_row(&admin(), "call-shared")
        .await
        .unwrap();
    assert_eq!(unscoped.storage_key, "call-shared");
    let foreign = unscoped.row.unwrap();
    assert_eq!(foreign.user.as_deref(), Some("mallory"));
    assert_eq!(foreign.messages, None);
    assert_eq!(foreign.metadata, None);
    finish(seeded).await;
}

#[tokio::test]
async fn ownership_moved_after_the_precheck_serves_no_row() {
    let seeded = seeded().await;
    let checks = postgres(&seeded, vec![user("alice", &ALICE_TEAMS)]);
    checks
        .assert_can_view_request_id(&alice(), "call-team")
        .await
        .unwrap();
    execute(
        &seeded.pool,
        r#"UPDATE "LiteLLM_SpendLogs" SET team_id = 'team-x' WHERE request_id = 'req-team'"#,
    )
    .await;
    let resolved = checks
        .resolve_payload_row(&alice(), "call-team")
        .await
        .unwrap();
    assert_eq!(resolved.row, None);
    assert_eq!(resolved.storage_key, "call-team");
    finish(seeded).await;
}

#[tokio::test]
async fn a_permission_removed_before_the_recheck_rejects_the_fetched_row() {
    let seeded = seeded().await;
    let racing = RacingStore {
        inner: PostgresVisibility {
            store: seeded.store.clone(),
        },
        pool: seeded.pool.clone(),
        after_teams: r#"UPDATE "LiteLLM_TeamTable" SET team_member_permissions = '{}' WHERE team_id = 'team-a'"#,
    };
    let checks = visibility(Arc::new(racing), vec![user("alice", &ALICE_TEAMS)]);
    let outcome = checks
        .resolve_payload_row(&alice(), "call-team")
        .await
        .map(|_| ());
    assert_eq!(forbidden(outcome), "call-team");
    finish(seeded).await;
}

#[tokio::test]
async fn failed_membership_reads_fall_back_to_own_rows() {
    let seeded = seeded().await;
    let unknown = postgres(&seeded, Vec::new());
    let lookup = unknown.permitted_team_ids(&alice()).await;
    assert!(matches!(lookup, Err(VisibilityError::Users(_))));
    assert!(unknown.permitted_teams_or_empty(&alice()).await.is_empty());
    let hidden = unknown
        .resolve_payload_row(&alice(), "call-team")
        .await
        .unwrap();
    assert_eq!(hidden.row, None);
    let checks = postgres(&seeded, vec![user("alice", &ALICE_TEAMS)]);
    execute(
        &seeded.pool,
        r#"ALTER TABLE "LiteLLM_TeamTable" RENAME TO "LiteLLM_TeamTable_moved""#,
    )
    .await;
    let teams = checks.permitted_team_ids(&alice()).await;
    assert!(matches!(teams, Err(VisibilityError::TeamList(_))));
    let own = checks
        .resolve_payload_row(&alice(), "call-shared")
        .await
        .unwrap();
    assert_eq!(own.storage_key, "req-own");
    let team_row = checks
        .resolve_payload_row(&alice(), "call-team")
        .await
        .unwrap();
    assert_eq!(team_row.row, None);
    let precheck = checks
        .assert_can_view_request_id(&alice(), "call-team")
        .await;
    assert!(matches!(precheck, Err(VisibilityError::Team(_))));
    finish(seeded).await;
}

#[tokio::test]
async fn a_null_user_id_is_never_an_unrestricted_viewer() {
    let seeded = seeded().await;
    let checks = postgres(&seeded, Vec::new());
    let nobody = caller(None, Some(UserRole::InternalUser));
    let resolved = checks
        .resolve_payload_row(&nobody, "req-anonymous")
        .await
        .unwrap();
    assert_eq!(resolved.row, None);
    assert_eq!(resolved.storage_key, "req-anonymous");
    let precheck = checks
        .assert_can_view_request_id(&nobody, "req-anonymous")
        .await;
    assert_eq!(forbidden(precheck), "req-anonymous");
    finish(seeded).await;
}

#[tokio::test]
async fn cold_payloads_are_checked_against_their_recorded_owner() {
    let seeded = seeded().await;
    let checks = postgres(&seeded, vec![user("alice", &ALICE_TEAMS)]);
    let owned = json(r#"{"metadata":{"user_api_key_user_id":"alice"}}"#);
    checks
        .assert_owns_cold_payload(&alice(), &owned, "id-1")
        .await
        .unwrap();
    let team = json(r#"{"metadata":{"user_api_key_user_id":7,"user_api_key_team_id":"team-a"}}"#);
    checks
        .assert_owns_cold_payload(&alice(), &team, "id-1")
        .await
        .unwrap();
    let foreign = json(r#"{"metadata":{"user_api_key_team_id":"team-x"}}"#);
    let denied = checks
        .assert_owns_cold_payload(&alice(), &foreign, "id-1")
        .await;
    assert_eq!(forbidden(denied), "id-1");
    let unowned = checks
        .assert_owns_cold_payload(&alice(), &json("{}"), "id-2")
        .await;
    assert_eq!(forbidden(unowned), "id-2");
    checks
        .assert_owns_cold_payload(&admin(), &foreign, "id-1")
        .await
        .unwrap();
    finish(seeded).await;
}
