#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_auth::user_roles::UserRole;
use betterlite_config::scim_role_settings::{Settings, load_scim_role_settings};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use super::data::{ScimUserData, extract_scim_user_data};
use super::metadata::build_scim_metadata;
use super::recompute::{ScimRoleError, recompute_scim_member_roles, scim_groups_from_team_ids};
use super::roles::{default_scim_user_role, resolve_scim_user_role};
use crate::scim_records::user::{User, UserGroup};

fn lookup() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn settings(source: &str) -> Settings {
    load_scim_role_settings(source, &lookup()).unwrap()
}

fn user(body: &str) -> User {
    serde_json::from_str(body).unwrap()
}

fn extracted(body: &str) -> ScimUserData {
    extract_scim_user_data(&user(body))
}

fn groups(body: &str) -> Vec<UserGroup> {
    serde_json::from_str(body).unwrap()
}

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn built(data: &ScimUserData, active: Option<bool>) -> JsonData {
    JsonData::Object(build_scim_metadata(data, active).unwrap())
}

#[test]
fn scim_role_settings_extract_keeps_the_first_email_even_when_another_is_primary() {
    let data = extracted(
        r#"{"schemas":[],"emails":[{"value":"first@example.com"},{"value":"second@example.com","primary":true}]}"#,
    );
    assert_eq!(data.user_email.as_deref(), Some("first@example.com"));
}

#[test]
fn scim_role_settings_extract_without_emails_has_no_address() {
    let data = extracted(r#"{"schemas":[]}"#);
    assert_eq!(data.user_email, None);
}

#[test]
fn scim_role_settings_extract_takes_the_alias_from_given_name_only() {
    let data = extracted(
        r#"{"schemas":[],"name":{"givenName":"Ann","familyName":"Lee","formatted":"Dr Ann"}}"#,
    );
    assert_eq!(data.user_alias.as_deref(), Some("Ann"));
    assert_eq!(data.given_name.as_deref(), Some("Ann"));
    assert_eq!(data.family_name.as_deref(), Some("Lee"));
}

#[test]
fn scim_role_settings_extract_without_a_given_name_has_no_alias() {
    let empty = extracted(r#"{"schemas":[],"name":{"givenName":""}}"#);
    assert_eq!(empty.user_alias, None);
    assert_eq!(empty.given_name.as_deref(), Some(""));
    let missing = extracted(r#"{"schemas":[],"name":{"familyName":"Lee"}}"#);
    assert_eq!(missing.user_alias, None);
    assert_eq!(missing.given_name, None);
    assert_eq!(missing.family_name.as_deref(), Some("Lee"));
}

#[test]
fn scim_role_settings_extract_carries_external_id_groups_and_directory_blocks() {
    let data = extracted(
        r#"{"schemas":[],"externalId":"sso-1","groups":[{"value":"t1","display":"alpha"},{"value":"t2"}],"entitlements":["read"],"roles":[],"active":false,"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User":{"department":"ops"}}"#,
    );
    assert_eq!(data.sso_user_id.as_deref(), Some("sso-1"));
    assert_eq!(data.teams, ["t1", "t2"]);
    assert_eq!(data.active, Some(false));
    assert_eq!(data.entitlements.unwrap().len(), 1);
    assert!(data.roles.unwrap().is_empty());
    assert_eq!(data.enterprise.unwrap().department.as_deref(), Some("ops"));
}

#[test]
fn scim_role_settings_extract_without_groups_or_blocks_leaves_them_empty() {
    let data = extracted(r#"{"schemas":[]}"#);
    assert!(data.teams.is_empty());
    assert!(data.entitlements.is_none());
    assert!(data.roles.is_none());
    assert!(data.enterprise.is_none());
    assert_eq!(data.active, Some(true));
}

#[test]
fn scim_role_settings_metadata_always_carries_both_names() {
    let data = extracted(r#"{"schemas":[]}"#);
    assert_eq!(
        built(&data, None),
        json(r#"{"scim_metadata":{"givenName":null,"familyName":null}}"#)
    );
    let named = extracted(r#"{"schemas":[],"name":{"givenName":"Ann","familyName":"Lee"}}"#);
    assert_eq!(
        built(&named, None),
        json(r#"{"scim_metadata":{"givenName":"Ann","familyName":"Lee"}}"#)
    );
}

#[test]
fn scim_role_settings_metadata_adds_active_only_when_supplied() {
    let data = extracted(r#"{"schemas":[]}"#);
    assert_eq!(
        built(&data, Some(false)),
        json(r#"{"scim_metadata":{"givenName":null,"familyName":null},"scim_active":false}"#)
    );
    assert_eq!(
        built(&data, None),
        json(r#"{"scim_metadata":{"givenName":null,"familyName":null}}"#)
    );
}

#[test]
fn scim_role_settings_metadata_writes_the_enterprise_block_by_alias() {
    let data = extracted(
        r#"{"schemas":[],"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User":{"employeeNumber":"7","costCenter":"cc","organization":"org","division":"div","department":"dep","manager":{"value":"m1","displayName":"M","$ref":"https://example.com/m1"}}}"#,
    );
    assert_eq!(
        built(&data, None),
        json(
            r#"{"scim_metadata":{"givenName":null,"familyName":null},"scim_enterprise":{"employeeNumber":"7","costCenter":"cc","organization":"org","division":"div","department":"dep","manager":{"value":"m1","displayName":"M","$ref":"https://example.com/m1"}}}"#,
        )
    );
}

#[test]
fn scim_role_settings_metadata_keeps_an_empty_enterprise_block() {
    let data = extracted(
        r#"{"schemas":[],"urn:ietf:params:scim:schemas:extension:enterprise:2.0:User":{}}"#,
    );
    assert_eq!(
        built(&data, None),
        json(r#"{"scim_metadata":{"givenName":null,"familyName":null},"scim_enterprise":{}}"#)
    );
}

#[test]
fn scim_role_settings_metadata_keeps_empty_directory_lists() {
    let data = extracted(r#"{"schemas":[],"entitlements":[],"roles":[]}"#);
    assert_eq!(
        built(&data, None),
        json(
            r#"{"scim_metadata":{"givenName":null,"familyName":null},"scim_entitlements":[],"scim_roles":[]}"#,
        )
    );
}

#[test]
fn scim_role_settings_metadata_dumps_each_directory_attribute() {
    let data = extracted(
        r#"{"schemas":[],"entitlements":["read",{"value":"write","display":"Write","type":"direct","primary":false,"division":"ops","note":null}],"roles":[{"value":"admin","primary":true}]}"#,
    );
    assert_eq!(
        built(&data, None),
        json(
            r#"{"scim_metadata":{"givenName":null,"familyName":null},"scim_entitlements":[{"value":"read"},{"value":"write","display":"Write","type":"direct","primary":false,"division":"ops"}],"scim_roles":[{"value":"admin","primary":true}]}"#,
        )
    );
}

#[test]
fn scim_role_settings_default_role_is_the_viewer_when_unset() {
    let loaded = settings("model_list: []\n");
    assert_eq!(
        default_scim_user_role(&loaded),
        UserRole::InternalUserViewer
    );
}

#[rstest]
#[case::proxy_admin("proxy_admin", UserRole::ProxyAdmin)]
#[case::proxy_admin_viewer("proxy_admin_viewer", UserRole::ProxyAdminViewer)]
#[case::internal_user("internal_user", UserRole::InternalUser)]
#[case::internal_user_viewer("internal_user_viewer", UserRole::InternalUserViewer)]
fn scim_role_settings_maps_every_configured_role(
    #[case] configured: &str,
    #[case] expected: UserRole,
) {
    let loaded = settings(&format!(
        "general_settings:\n  default_internal_user_params:\n    user_role: {configured}\n"
    ));
    assert_eq!(default_scim_user_role(&loaded), expected);
}

#[test]
fn scim_role_settings_resolve_leaves_the_role_alone_without_an_admin_group() {
    let stored = groups(r#"[{"value":"SCIM Admins"},{"value":"t2","display":"SCIM Admins"}]"#);
    assert_eq!(
        resolve_scim_user_role(&stored, None, UserRole::InternalUserViewer),
        None
    );
}

#[test]
fn scim_role_settings_resolve_grants_admin_by_group_value() {
    let stored = groups(r#"[{"value":"t1","display":"alpha"},{"value":"SCIM Admins"}]"#);
    assert_eq!(
        resolve_scim_user_role(&stored, Some("SCIM Admins"), UserRole::InternalUser),
        Some(UserRole::ProxyAdmin)
    );
}

#[test]
fn scim_role_settings_resolve_grants_admin_by_group_display() {
    let stored = groups(r#"[{"value":"team-42","display":"SCIM Admins"}]"#);
    assert_eq!(
        resolve_scim_user_role(&stored, Some("SCIM Admins"), UserRole::InternalUser),
        Some(UserRole::ProxyAdmin)
    );
}

#[test]
fn scim_role_settings_resolve_falls_back_to_the_default_role() {
    let outside = groups(r#"[{"value":"team-42","display":"Engineering"},{"value":"team-43"}]"#);
    assert_eq!(
        resolve_scim_user_role(&outside, Some("SCIM Admins"), UserRole::InternalUser),
        Some(UserRole::InternalUser)
    );
    assert_eq!(
        resolve_scim_user_role(&[], Some("SCIM Admins"), UserRole::InternalUser),
        Some(UserRole::InternalUser)
    );
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_UserTable" (
    user_id TEXT PRIMARY KEY,
    user_alias TEXT,
    user_email TEXT,
    teams TEXT[] NOT NULL DEFAULT '{}',
    metadata JSONB,
    user_role TEXT,
    created_at TIMESTAMP(3),
    updated_at TIMESTAMP(3)
);
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    team_alias TEXT,
    members_with_roles JSONB NOT NULL DEFAULT '[]'
);
INSERT INTO "LiteLLM_UserTable" (user_id, teams, user_role) VALUES
    ('u-member', ARRAY['t-admin'], 'internal_user_viewer'),
    ('u-other', ARRAY['t-plain'], 'proxy_admin'),
    ('u-value', ARRAY['SCIM Admins'], 'internal_user_viewer'),
    ('u-none', ARRAY['t-null'], 'internal_user'),
    ('u-idle', ARRAY['t-admin'], 'internal_user');
INSERT INTO "LiteLLM_TeamTable" (team_id, team_alias) VALUES
    ('t-admin', 'SCIM Admins'),
    ('t-plain', 'Engineering'),
    ('t-null', NULL);
"#;

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["scim-role-settings-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    let client = pool.get().await.unwrap();
    client.batch_execute(SCHEMA).await.unwrap();
    drop(client);
    (fixture, pool)
}

fn store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

async fn stored_role(pool: &Pool, user_id: &str) -> Option<String> {
    let client = pool.get().await.unwrap();
    let row = client
        .query_one(
            r#"SELECT user_role FROM "LiteLLM_UserTable" WHERE user_id = $1"#,
            &[&user_id],
        )
        .await
        .unwrap();
    row.get(0)
}

#[tokio::test]
async fn scim_role_settings_groups_from_team_ids_keep_missing_teams() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let ids = ["t-admin", "missing", "t-null"].map(str::to_owned);
    let built = scim_groups_from_team_ids(&store, &ids).await.unwrap();
    assert_eq!(built.len(), 3);
    assert_eq!(built[0].value, "t-admin");
    assert_eq!(built[0].display.as_deref(), Some("SCIM Admins"));
    assert_eq!(built[1].value, "missing");
    assert_eq!(built[1].display, None);
    assert_eq!(built[2].value, "t-null");
    assert_eq!(built[2].display, None);
    assert_eq!(built[2].kind.0.as_deref(), Some("direct"));
    let empty = scim_groups_from_team_ids(&store, &[]).await.unwrap();
    assert!(empty.is_empty());
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_role_settings_recompute_promotes_demotes_and_skips_missing_users() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let loaded = settings("litellm_settings:\n  scim_admin_group: SCIM Admins\n");
    let ids = ["u-member", "u-other", "u-value", "u-none", "ghost"].map(str::to_owned);
    recompute_scim_member_roles(&store, &ids, &loaded)
        .await
        .unwrap();
    assert_eq!(
        stored_role(&pool, "u-member").await.as_deref(),
        Some("proxy_admin")
    );
    assert_eq!(
        stored_role(&pool, "u-other").await.as_deref(),
        Some("internal_user_viewer")
    );
    assert_eq!(
        stored_role(&pool, "u-value").await.as_deref(),
        Some("proxy_admin")
    );
    assert_eq!(
        stored_role(&pool, "u-none").await.as_deref(),
        Some("internal_user_viewer")
    );
    assert_eq!(
        stored_role(&pool, "u-idle").await.as_deref(),
        Some("internal_user")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_role_settings_recompute_does_nothing_without_an_admin_group() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let loaded = settings("model_list: []\n");
    let ids = ["u-member", "u-other"].map(str::to_owned);
    recompute_scim_member_roles(&store, &ids, &loaded)
        .await
        .unwrap();
    assert_eq!(
        stored_role(&pool, "u-member").await.as_deref(),
        Some("internal_user_viewer")
    );
    assert_eq!(
        stored_role(&pool, "u-other").await.as_deref(),
        Some("proxy_admin")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_role_settings_recompute_uses_the_configured_default_role() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let loaded = settings(
        "litellm_settings:\n  scim_admin_group: SCIM Admins\ngeneral_settings:\n  default_internal_user_params:\n    user_role: internal_user\n",
    );
    let ids = ["u-other", "u-none", "u-member"].map(str::to_owned);
    recompute_scim_member_roles(&store, &ids, &loaded)
        .await
        .unwrap();
    assert_eq!(
        stored_role(&pool, "u-other").await.as_deref(),
        Some("internal_user")
    );
    assert_eq!(
        stored_role(&pool, "u-none").await.as_deref(),
        Some("internal_user")
    );
    assert_eq!(
        stored_role(&pool, "u-member").await.as_deref(),
        Some("proxy_admin")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn scim_role_settings_recompute_of_no_users_is_a_no_op() {
    let missing = PostgresStore { pool: None };
    let loaded = settings("litellm_settings:\n  scim_admin_group: SCIM Admins\n");
    recompute_scim_member_roles(&missing, &[], &loaded)
        .await
        .unwrap();
}

#[tokio::test]
async fn scim_role_settings_recompute_without_a_database_reports_the_failure() {
    let missing = PostgresStore { pool: None };
    let loaded = settings("litellm_settings:\n  scim_admin_group: SCIM Admins\n");
    let ids = ["u-member"].map(str::to_owned);
    let error = recompute_scim_member_roles(&missing, &ids, &loaded)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ScimRoleError::Database { .. }));
}

#[tokio::test]
async fn scim_role_settings_groups_without_a_database_report_the_team_failure() {
    let missing = PostgresStore { pool: None };
    let ids = ["t-admin"].map(str::to_owned);
    let error = scim_groups_from_team_ids(&missing, &ids)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ScimRoleError::Team { .. }));
}
