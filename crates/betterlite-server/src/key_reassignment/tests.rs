#![cfg(test)]

mod support;

use std::sync::atomic::Ordering;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::PermissionList;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;

use self::support::{Database, caller, caller_policy, key_row, team_row};

use crate::key_access::error::DetailShape;
use crate::key_fields::shared::hash_token_if_needed;
use crate::key_patch_fields::error::KeyPatchError;
use crate::key_reassignment::end_user_budget::validate_end_user_budget_id_change;
use crate::key_reassignment::error::KeyReassignmentError;
use crate::key_reassignment::existing_key::get_and_validate_existing_key;
use crate::key_reassignment::team_change::{TeamChangeDeps, validate_key_team_change};
use crate::team_access::error::TeamAccessError;

const ADMIN: &str = r#"{"user_id": "admin", "user_role": "proxy_admin"}"#;
const WORKER: &str = r#"{"user_id": "worker", "user_role": "internal_user"}"#;

#[tokio::test]
async fn key_no_database_500() {
    let error = get_and_validate_existing_key(None, None, None)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::KeyPatch(KeyPatchError::DatabaseNotConnected)
    ));
    assert_eq!(error.response(), (Some(500_u16), Some(DetailShape::Error)));
    assert_eq!(error.to_string(), "Database not connected");
    assert_eq!(error.proxy_exception(), None);
}

#[tokio::test]
async fn token_precedes_alias() {
    let hashed = hash_token_if_needed("sk-live");
    let seed = format!(
        r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "user_id") VALUES ('{hashed}', 'shared', 'u-live');
           INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "user_id") VALUES ('other', 'shared', 'u-other');"#
    );
    let database = Database::new(&seed).await;
    let row =
        get_and_validate_existing_key(Some("sk-live"), Some("shared"), Some(database.client()))
            .await
            .unwrap();
    assert_eq!(row.user_id.as_deref(), Some("u-live"));
    let missing = get_and_validate_existing_key(Some(""), Some("shared"), Some(database.client()))
        .await
        .unwrap_err();
    assert!(matches!(
        &missing,
        KeyReassignmentError::KeyPatch(KeyPatchError::KeyNotFound)
    ));
    database.close().await;
}

#[tokio::test]
async fn token_relation_loaded() {
    let hashed = hash_token_if_needed("sk-live");
    let seed = format!(
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" ("object_permission_id", "models") VALUES ('op-1', ARRAY['gpt-4']);
           INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "object_permission_id") VALUES ('{hashed}', 'live', 'op-1');"#
    );
    let database = Database::new(&seed).await;
    let row = get_and_validate_existing_key(Some("sk-live"), None, Some(database.client()))
        .await
        .unwrap();
    let relation = row
        .object_permission
        .expect("the token lookup loads the relation");
    assert_eq!(relation.object_permission_id, "op-1");
    assert_eq!(
        relation.models,
        PermissionList(Some(vec![String::from("gpt-4")]))
    );
    database.close().await;
}

#[tokio::test]
async fn key_missing_404() {
    let database = Database::new("").await;
    let error = get_and_validate_existing_key(Some("sk-absent"), None, Some(database.client()))
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::KeyPatch(KeyPatchError::KeyNotFound)
    ));
    assert_eq!(error.response(), (Some(404_u16), Some(DetailShape::Text)));
    let exception = error
        .proxy_exception()
        .expect("a missing key keeps its ProxyException");
    assert_eq!(exception.message, "Key not found.");
    assert_eq!(exception.error_type, "not_found_error");
    assert_eq!(exception.param.as_deref(), Some("key"));
    assert_eq!(exception.code, "404");
    database.close().await;
}

#[tokio::test]
async fn identity_absent_400() {
    let database = Database::new("").await;
    let error = get_and_validate_existing_key(None, None, Some(database.client()))
        .await
        .unwrap_err();
    assert!(matches!(&error, KeyReassignmentError::MissingKeyIdentifier));
    assert_eq!(error.response(), (Some(400_u16), Some(DetailShape::Text)));
    let exception = error
        .proxy_exception()
        .expect("an absent identifier keeps its ProxyException");
    assert_eq!(
        exception.message,
        "either key or key_alias must be provided"
    );
    assert_eq!(exception.error_type, "bad_request_error");
    assert_eq!(exception.param.as_deref(), Some("key"));
    assert_eq!(exception.code, "400");
    database.close().await;
}

#[tokio::test]
async fn alias_missing_404() {
    let database = Database::new("").await;
    let error = get_and_validate_existing_key(None, Some("nope"), Some(database.client()))
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::AliasNotFound { alias } if alias.as_str() == "nope"
    ));
    assert_eq!(error.response(), (Some(404_u16), Some(DetailShape::Text)));
    let exception = error
        .proxy_exception()
        .expect("a missing alias keeps its ProxyException");
    assert_eq!(
        exception.message,
        "Key not found. No key with key_alias='nope'."
    );
    assert_eq!(exception.error_type, "not_found_error");
    assert_eq!(exception.param.as_deref(), Some("key_alias"));
    assert_eq!(exception.code, "404");
    database.close().await;
}

#[tokio::test]
async fn alias_ambiguous_400() {
    let seed = r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias") VALUES ('one', 'dup'), ('two', 'dup');"#;
    let database = Database::new(seed).await;
    let error = get_and_validate_existing_key(None, Some("dup"), Some(database.client()))
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::AliasAmbiguous { alias } if alias.as_str() == "dup"
    ));
    assert_eq!(error.response(), (Some(400_u16), Some(DetailShape::Text)));
    let exception = error
        .proxy_exception()
        .expect("an ambiguous alias keeps its ProxyException");
    assert_eq!(
        exception.message,
        "Multiple keys share key_alias='dup', so it cannot be used as an identifier."
    );
    assert_eq!(exception.error_type, "bad_request_error");
    assert_eq!(exception.param.as_deref(), Some("key_alias"));
    assert_eq!(exception.code, "400");
    database.close().await;
}

#[tokio::test]
async fn alias_single_no_relation() {
    let seed = r#"INSERT INTO "LiteLLM_ObjectPermissionTable" ("object_permission_id") VALUES ('op-1');
                   INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "object_permission_id") VALUES ('solo', 'solo-alias', 'op-1');"#;
    let database = Database::new(seed).await;
    let row = get_and_validate_existing_key(None, Some("solo-alias"), Some(database.client()))
        .await
        .unwrap();
    assert_eq!(row.key_alias.as_deref(), Some("solo-alias"));
    assert_eq!(row.object_permission_id.as_deref(), Some("op-1"));
    assert!(row.object_permission.is_none());
    database.close().await;
}

#[tokio::test]
async fn reads_route_to_reader() {
    let reader_hash = hash_token_if_needed("sk-reader");
    let reader = Database::new(&format!(
        r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "user_id") VALUES ('{reader_hash}', 'reader-alias', 'reader-user');"#
    ))
    .await;
    let writer_hash = hash_token_if_needed("sk-writer");
    let writer = Database::new(&format!(
        r#"INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "user_id") VALUES ('{writer_hash}', 'writer-alias', 'writer-user');"#
    ))
    .await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer.store,
        reader: &reader.store,
        availability: &availability,
    };
    let by_token = get_and_validate_existing_key(Some("sk-reader"), None, Some(client))
        .await
        .unwrap();
    assert_eq!(by_token.user_id.as_deref(), Some("reader-user"));
    let by_alias = get_and_validate_existing_key(None, Some("reader-alias"), Some(client))
        .await
        .unwrap();
    assert_eq!(by_alias.user_id.as_deref(), Some("reader-user"));
    let absent = get_and_validate_existing_key(Some("sk-writer"), None, Some(client))
        .await
        .unwrap_err();
    assert!(matches!(
        &absent,
        KeyReassignmentError::KeyPatch(KeyPatchError::KeyNotFound)
    ));
    let absent_alias = get_and_validate_existing_key(None, Some("writer-alias"), Some(client))
        .await
        .unwrap_err();
    assert!(matches!(
        &absent_alias,
        KeyReassignmentError::AliasNotFound { .. }
    ));
    writer.close().await;
    reader.close().await;
}

#[tokio::test]
async fn writer_outage_reads_reader() {
    let hashed = hash_token_if_needed("sk-live");
    let reader = Database::new(&format!(
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" ("object_permission_id", "models") VALUES ('op-1', ARRAY['gpt-4']);
           INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "object_permission_id") VALUES ('{hashed}', 'live', 'op-1');"#
    ))
    .await;
    let availability = Availability::default();
    availability
        .writer_unavailable
        .store(true, Ordering::SeqCst);
    let unavailable = PostgresStore { pool: None };
    let client = PrismaClient {
        writer: &unavailable,
        reader: &reader.store,
        availability: &availability,
    };
    let row = get_and_validate_existing_key(Some("sk-live"), None, Some(client))
        .await
        .unwrap();
    let relation = row
        .object_permission
        .expect("the token lookup loads the relation");
    assert_eq!(
        relation.models,
        PermissionList(Some(vec![String::from("gpt-4")]))
    );
    let error = get_and_validate_existing_key(Some("sk-absent"), None, Some(client))
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::KeyPatch(KeyPatchError::KeyNotFound)
    ));
    assert_eq!(error.response(), (Some(404_u16), Some(DetailShape::Text)));
    let alias_error = get_and_validate_existing_key(None, Some("absent"), Some(client))
        .await
        .unwrap_err();
    assert!(matches!(
        &alias_error,
        KeyReassignmentError::AliasNotFound { .. }
    ));
    assert_eq!(
        alias_error.response(),
        (Some(404_u16), Some(DetailShape::Text))
    );
    reader.close().await;
}

#[tokio::test]
async fn reader_fallback_reads_writer() {
    let hashed = hash_token_if_needed("sk-live");
    let writer = Database::new(&format!(
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" ("object_permission_id", "models") VALUES ('op-1', ARRAY['gpt-4']);
           INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "user_id", "object_permission_id") VALUES ('{hashed}', 'live', 'writer-user', 'op-1');"#
    ))
    .await;
    let reader = Database::new("").await;
    let availability = Availability::default();
    availability
        .reader_unavailable
        .store(true, Ordering::SeqCst);
    let client = PrismaClient {
        writer: &writer.store,
        reader: &reader.store,
        availability: &availability,
    };
    let row = get_and_validate_existing_key(Some("sk-live"), None, Some(client))
        .await
        .unwrap();
    assert_eq!(row.user_id.as_deref(), Some("writer-user"));
    let relation = row
        .object_permission
        .expect("the fallback still loads the relation");
    assert_eq!(
        relation.models,
        PermissionList(Some(vec![String::from("gpt-4")]))
    );
    let by_alias = get_and_validate_existing_key(None, Some("live"), Some(client))
        .await
        .unwrap();
    assert_eq!(by_alias.user_id.as_deref(), Some("writer-user"));
    reader.close().await;
    writer.close().await;
}

#[tokio::test]
async fn relation_follows_row_side() {
    let hashed = hash_token_if_needed("sk-live");
    let reader = Database::new(&format!(
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" ("object_permission_id", "models") VALUES ('op-1', ARRAY['gpt-4']);
           INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "user_id", "object_permission_id") VALUES ('{hashed}', 'live', 'reader-user', 'op-1');"#
    ))
    .await;
    let writer = Database::new(&format!(
        r#"INSERT INTO "LiteLLM_ObjectPermissionTable" ("object_permission_id", "models") VALUES ('op-1', ARRAY['claude']);
           INSERT INTO "LiteLLM_VerificationToken" ("token", "key_alias", "user_id", "object_permission_id") VALUES ('{hashed}', 'live', 'writer-user', 'op-1');"#
    ))
    .await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer.store,
        reader: &reader.store,
        availability: &availability,
    };
    let row = get_and_validate_existing_key(Some("sk-live"), None, Some(client))
        .await
        .unwrap();
    assert_eq!(row.user_id.as_deref(), Some("reader-user"));
    let relation = row
        .object_permission
        .expect("the token lookup loads the relation");
    assert_eq!(
        relation.models,
        PermissionList(Some(vec![String::from("gpt-4")]))
    );
    writer.close().await;
    reader.close().await;
}

#[tokio::test]
async fn unchanged_budget_noop() {
    let worker = caller(WORKER);
    validate_end_user_budget_id_change(Some("b-1"), Some("b-1"), &worker, None)
        .await
        .unwrap();
    validate_end_user_budget_id_change(None, Some("b-1"), &worker, None)
        .await
        .unwrap();
    validate_end_user_budget_id_change(None, None, &worker, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn empty_existing_budget_noop() {
    let worker = caller(WORKER);
    validate_end_user_budget_id_change(Some(""), None, &worker, None)
        .await
        .unwrap();
    validate_end_user_budget_id_change(Some(""), Some(""), &worker, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn nonadmin_budget_change_403() {
    let worker = caller(WORKER);
    let error = validate_end_user_budget_id_change(Some("b-2"), Some("b-1"), &worker, None)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::EndUserBudgetForbidden
    ));
    assert_eq!(error.response(), (Some(403_u16), Some(DetailShape::Error)));
    assert_eq!(
        error.to_string(),
        "Only proxy admins can set end_user_budget_id on a key."
    );
    assert_eq!(error.proxy_exception(), None);
}

#[tokio::test]
async fn admin_budget_clear_no_query() {
    let admin = caller(ADMIN);
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };
    validate_end_user_budget_id_change(Some(""), Some("b-1"), &admin, Some(client))
        .await
        .unwrap();
}

#[tokio::test]
async fn end_user_budget_unknown() {
    let database =
        Database::new(r#"INSERT INTO "LiteLLM_BudgetTable" ("budget_id") VALUES ('b-1');"#).await;
    let admin = caller(ADMIN);
    validate_end_user_budget_id_change(Some("b-1"), None, &admin, Some(database.client()))
        .await
        .unwrap();
    let error = validate_end_user_budget_id_change(
        Some("b-2"),
        Some("b-1"),
        &admin,
        Some(database.client()),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::EndUserBudgetUnknown { budget_id } if budget_id.as_str() == "b-2"
    ));
    assert_eq!(error.response(), (Some(400_u16), Some(DetailShape::Error)));
    assert_eq!(
        error.to_string(),
        "end_user_budget_id=b-2 does not match any budget."
    );
    assert_eq!(error.proxy_exception(), None);
    database.close().await;
}

#[tokio::test]
async fn budget_change_without_database_500() {
    let admin = caller(ADMIN);
    let error = validate_end_user_budget_id_change(Some("b-1"), None, &admin, None)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::KeyPatch(KeyPatchError::DatabaseNotConnected)
    ));
    assert_eq!(error.response(), (Some(500_u16), Some(DetailShape::Error)));
}

#[tokio::test]
async fn all_team_models_skipped() {
    let key = key_row(r#"{"models": ["all-team-models"]}"#);
    let team = team_row(r#"{"team_id": "team-1", "models": ["gpt-4"]}"#);
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap();
}

#[tokio::test]
async fn team_model_denial_first() {
    let key = key_row(r#"{"models": ["claude"], "tpm_limit": 200, "user_id": "ghost"}"#);
    let team = team_row(
        r#"{"team_id": "team-1", "models": ["gpt-4"], "tpm_limit": 100, "members_with_roles": [{"user_id": "member", "role": "admin"}]}"#,
    );
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    let error = validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap_err();
    assert!(matches!(&error, KeyReassignmentError::ModelAccess(_)));
    assert_eq!(error.response(), (Some(403_u16), None));
    let exception = error
        .proxy_exception()
        .expect("a model denial keeps its ProxyException");
    assert_eq!(exception.error_type, "team_model_access_denied");
    assert_eq!(exception.param.as_deref(), Some("model"));
    assert_eq!(exception.code, "403");
}

#[tokio::test]
async fn tpm_excess_403() {
    let key = key_row(r#"{"tpm_limit": 200, "user_id": "ghost"}"#);
    let team = team_row(
        r#"{"team_id": "team-1", "tpm_limit": 100, "members_with_roles": [{"user_id": "member", "role": "admin"}]}"#,
    );
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    let error = validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::TpmExceedsTeam {
            key_tpm: 200,
            team_tpm: 100
        }
    ));
    assert_eq!(error.response(), (Some(403_u16), Some(DetailShape::Text)));
    assert_eq!(
        error.to_string(),
        "Key has a tpm_limit=200 which is greater than the team's tpm_limit=100."
    );
    assert_eq!(error.proxy_exception(), None);
}

#[tokio::test]
async fn rpm_excess_403() {
    let key = key_row(r#"{"tpm_limit": 5, "rpm_limit": 300, "user_id": "ghost"}"#);
    let team = team_row(
        r#"{"team_id": "team-1", "tpm_limit": 100, "rpm_limit": 200, "members_with_roles": [{"user_id": "member", "role": "admin"}]}"#,
    );
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    let error = validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::RpmExceedsTeam {
            key_rpm: 300,
            team_rpm: 200
        }
    ));
    assert_eq!(error.response(), (Some(403_u16), Some(DetailShape::Text)));
    assert_eq!(
        error.to_string(),
        "Key has a rpm_limit=300 which is greater than the team's rpm_limit=200."
    );
    assert_eq!(error.proxy_exception(), None);
}

#[tokio::test]
async fn absent_tpm_skips_rpm() {
    let key = key_row(r#"{"rpm_limit": 300}"#);
    let team = team_row(r#"{"team_id": "team-1", "tpm_limit": 100, "rpm_limit": 200}"#);
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap();
}

#[tokio::test]
async fn zero_team_rates_skipped() {
    let key = key_row(r#"{"tpm_limit": 300, "rpm_limit": 300}"#);
    let team = team_row(r#"{"team_id": "team-1", "tpm_limit": 0, "rpm_limit": 0}"#);
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap();
}

#[tokio::test]
async fn equal_rates_allowed() {
    let key = key_row(r#"{"tpm_limit": 100, "rpm_limit": 100}"#);
    let team = team_row(r#"{"team_id": "team-1", "tpm_limit": 100, "rpm_limit": 100}"#);
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap();
}

#[tokio::test]
async fn owner_membership_required() {
    let key = key_row(r#"{"user_id": "ghost"}"#);
    let team = team_row(
        r#"{"team_id": "team-1", "members_with_roles": [{"user_id": "member", "role": "admin"}]}"#,
    );
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    let error = validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::OwnerNotInTeam { user_id, team_id } if user_id.as_str() == "ghost" && team_id.as_str() == "team-1"
    ));
    assert_eq!(error.response(), (Some(403_u16), Some(DetailShape::Text)));
    assert_eq!(
        error.to_string(),
        "User=ghost is not a member of the team=team-1. Check team members via `/team/info`."
    );
}

#[tokio::test]
async fn null_owner_final_policy() {
    let key = key_row("{}");
    let team = team_row(
        r#"{"team_id": "team-1", "members_with_roles": [{"user_id": "boss", "role": "admin"}]}"#,
    );
    let worker = caller(WORKER);
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    let error = validate_key_team_change(&key, &team, &worker, &deps)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::InitiatorNotAdmin { user_id, team_id } if user_id.as_str() == "worker" && team_id.as_str() == "team-1"
    ));
    assert_eq!(error.response(), (Some(403_u16), Some(DetailShape::Text)));
    assert_eq!(
        error.to_string(),
        "User=worker is not a Proxy Admin or Team Admin for team=team-1. Please ask your Proxy Admin to allow this action under 'Member Permissions' for this team."
    );
    validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap();
}

#[tokio::test]
async fn initiator_admin_allowed() {
    let key = key_row(r#"{"user_id": "owner"}"#);
    let team = team_row(
        r#"{"team_id": "team-1", "members_with_roles": [{"user_id": "owner", "role": "user"}]}"#,
    );
    let worker = caller(WORKER);
    let admin = caller(ADMIN);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    let denied = validate_key_team_change(&key, &team, &worker, &deps)
        .await
        .unwrap_err();
    assert!(matches!(
        &denied,
        KeyReassignmentError::TeamAccess(TeamAccessError::MemberRoute { .. })
    ));
    let exception = denied
        .proxy_exception()
        .expect("a denied member route keeps its ProxyException");
    assert_eq!(exception.error_type, "team_member_permission_error");
    assert_eq!(exception.param.as_deref(), Some("/key/update"));
    assert_eq!(exception.code, "401");
    validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap();
}

#[tokio::test]
async fn owner_role_endpoint_permission() {
    let key = key_row(r#"{"user_id": "owner"}"#);
    let team = team_row(
        r#"{"team_id": "team-1", "members_with_roles": [{"user_id": "owner", "role": "user"}], "team_member_permissions": ["/key/update"]}"#,
    );
    let worker = caller(WORKER);
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    validate_key_team_change(&key, &team, &worker, &deps)
        .await
        .unwrap();
}

#[tokio::test]
async fn rate_denial_does_not_echo_token() {
    let policy = caller_policy();
    let context = policy.context();
    let deps = TeamChangeDeps {
        access: &policy.access,
        teams: None,
        context: &context,
    };
    let admin = caller(ADMIN);
    let key = key_row(r#"{"token": "sk-super-secret", "tpm_limit": 200}"#);
    let team = team_row(r#"{"team_id": "team-1", "tpm_limit": 100}"#);
    let error = validate_key_team_change(&key, &team, &admin, &deps)
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        KeyReassignmentError::TpmExceedsTeam { .. }
    ));
    let message = error.to_string();
    assert!(!message.contains("sk-super-secret"));
    assert!(message.contains("tpm_limit=200"));
    let rpm_key = key_row(r#"{"token": "sk-super-secret", "tpm_limit": 5, "rpm_limit": 300}"#);
    let rpm_team = team_row(r#"{"team_id": "team-1", "tpm_limit": 100, "rpm_limit": 200}"#);
    let rpm_error = validate_key_team_change(&rpm_key, &rpm_team, &admin, &deps)
        .await
        .unwrap_err();
    assert!(matches!(
        &rpm_error,
        KeyReassignmentError::RpmExceedsTeam { .. }
    ));
    let rpm_message = rpm_error.to_string();
    assert!(!rpm_message.contains("sk-super-secret"));
    assert!(rpm_message.contains("rpm_limit=300"));
}
