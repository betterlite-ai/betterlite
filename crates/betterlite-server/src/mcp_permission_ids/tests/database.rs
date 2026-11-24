use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use rstest::rstest;

use crate::mcp_permission_ids::store::{
    ExistingServersError, existing_object_permission_mcp_servers,
};

use super::fixtures::database::TestDatabase;
use super::fixtures::strings;

#[tokio::test]
async fn existing_grants_falsey_missing_and_duplicates() {
    let database = TestDatabase::start().await;
    let without_client = existing_object_permission_mcp_servers(Some("perm-1"), None).await;
    assert!(without_client.unwrap().is_empty());
    let without_id = existing_object_permission_mcp_servers(None, Some(database.prisma())).await;
    assert!(without_id.unwrap().is_empty());
    let empty_id = existing_object_permission_mcp_servers(Some(""), Some(database.prisma())).await;
    assert!(empty_id.unwrap().is_empty());
    let absent_row =
        existing_object_permission_mcp_servers(Some("absent"), Some(database.prisma())).await;
    assert!(absent_row.unwrap().is_empty());
    database
        .seed_permission_servers("perm-null", &Option::<Vec<String>>::None)
        .await;
    let null_servers =
        existing_object_permission_mcp_servers(Some("perm-null"), Some(database.prisma())).await;
    assert!(null_servers.unwrap().is_empty());
    database
        .seed_permission_servers("perm-servers", &strings(&["alias-a", "srv-1", "alias-a"]))
        .await;
    let stored =
        existing_object_permission_mcp_servers(Some("perm-servers"), Some(database.prisma()))
            .await
            .unwrap();
    assert_eq!(stored, strings(&["alias-a", "srv-1", "alias-a"]));
    database.close().await;
}

#[tokio::test]
async fn absent_servers_column_reads_empty() {
    let database = TestDatabase::start().await;
    database.drop_servers_column().await;
    database.seed_permission_without_servers("perm-1").await;
    let stored = existing_object_permission_mcp_servers(Some("perm-1"), Some(database.prisma()))
        .await
        .unwrap();
    assert!(stored.is_empty());
    database.close().await;
}

#[rstest]
#[case::text(r#""text""#)]
#[case::number("7")]
#[case::boolean("true")]
#[case::object(r#"{"a": []}"#)]
#[case::mixed_array(r#"["a", 1]"#)]
#[case::null_element(r#"["a", null]"#)]
#[tokio::test]
async fn wrong_shaped_servers_error(#[case] stored: &str) {
    let database = TestDatabase::start().await;
    database.alter_servers_to_jsonb().await;
    database
        .seed_permission_json_servers("perm-1", stored)
        .await;
    let outcome =
        existing_object_permission_mcp_servers(Some("perm-1"), Some(database.prisma())).await;
    assert!(matches!(
        outcome,
        Err(ExistingServersError::ServersNotAList)
    ));
    database.close().await;
}

#[tokio::test]
async fn null_array_element_errors() {
    let database = TestDatabase::start().await;
    let servers = vec![Some(String::from("srv-1")), None];
    database.seed_permission_servers("perm-1", &servers).await;
    let outcome =
        existing_object_permission_mcp_servers(Some("perm-1"), Some(database.prisma())).await;
    assert!(matches!(
        outcome,
        Err(ExistingServersError::ServersNotAList)
    ));
    database.close().await;
}

#[tokio::test]
async fn offline_or_absent_table_is_a_database_error() {
    let store = PostgresStore { pool: None };
    let availability = Availability::default();
    let prisma = PrismaClient {
        writer: &store,
        reader: &store,
        availability: &availability,
    };
    let offline = existing_object_permission_mcp_servers(Some("perm-1"), Some(prisma)).await;
    assert!(matches!(offline, Err(ExistingServersError::Database(_))));
    let database = TestDatabase::start().await;
    database.drop_permission_table().await;
    let absent =
        existing_object_permission_mcp_servers(Some("perm-1"), Some(database.prisma())).await;
    assert!(matches!(absent, Err(ExistingServersError::Database(_))));
    database.close().await;
}
