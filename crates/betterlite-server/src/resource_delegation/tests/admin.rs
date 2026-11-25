use betterlite_db::auxiliary_repository::repository::PrismaClient;

use crate::mcp_permission_ids::team::ALL_PROXY_MCP_SERVERS;
use crate::resource_delegation::admin_grant::{AdminGrantError, guard_all_proxy_team_grant};

use super::fixtures::admin_detail;
use super::fixtures::database::TestDatabase;
use super::fixtures::strings;

const ONLY_ADMIN: &str =
    "Only a proxy admin can grant a team access to all proxy MCP servers ('all-proxy-mcpservers').";

async fn guard(
    requested: &[String],
    stored_id: Option<&str>,
    is_proxy_admin: bool,
    client: Option<PrismaClient<'_>>,
) -> Result<(), AdminGrantError> {
    guard_all_proxy_team_grant(Some(requested), stored_id, is_proxy_admin, client).await
}

#[tokio::test]
async fn all_proxy_new_vs_stored() {
    let database = TestDatabase::start().await;
    database
        .seed_permission_servers("perm-sentinel", &strings(&[ALL_PROXY_MCP_SERVERS]))
        .await;
    database
        .seed_permission_servers("perm-other", &strings(&["srv-a"]))
        .await;
    let requested = strings(&[ALL_PROXY_MCP_SERVERS, "srv-a"]);
    guard(
        &requested,
        Some("perm-sentinel"),
        false,
        Some(database.prisma()),
    )
    .await
    .unwrap();
    let denied = guard(
        &requested,
        Some("perm-other"),
        false,
        Some(database.prisma()),
    )
    .await
    .unwrap_err();
    assert_eq!(admin_detail(denied), ONLY_ADMIN);
    database.close().await;
}

#[tokio::test]
async fn all_proxy_unresolvable_stored_id_is_denied() {
    let database = TestDatabase::start().await;
    let requested = strings(&[ALL_PROXY_MCP_SERVERS, "srv-a"]);
    let missing = guard(
        &requested,
        Some("perm-missing"),
        false,
        Some(database.prisma()),
    )
    .await
    .unwrap_err();
    assert_eq!(admin_detail(missing), ONLY_ADMIN);
    let empty_id = guard(&requested, Some(""), false, Some(database.prisma()))
        .await
        .unwrap_err();
    assert_eq!(admin_detail(empty_id), ONLY_ADMIN);
    let absent_id = guard(&requested, None, false, Some(database.prisma()))
        .await
        .unwrap_err();
    assert_eq!(admin_detail(absent_id), ONLY_ADMIN);
    let no_database = guard(&requested, Some("perm-sentinel"), false, None)
        .await
        .unwrap_err();
    assert_eq!(admin_detail(no_database), ONLY_ADMIN);
    database.close().await;
}

#[tokio::test]
async fn all_proxy_admin_short_circuit() {
    let requested = strings(&[ALL_PROXY_MCP_SERVERS]);
    guard(&requested, Some("perm-sentinel"), true, None)
        .await
        .unwrap();
    let other = strings(&["srv-a"]);
    guard(&other, None, false, None).await.unwrap();
    guard_all_proxy_team_grant(None, Some("perm-sentinel"), false, None)
        .await
        .unwrap();
}
