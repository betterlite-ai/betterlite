use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::managed_resource_rows::skills::SkillTable;
use betterlite_db::postgres_store::session::PostgresStore;
use deadpool_postgres::GenericClient as _;

use super::super::repository::PrismaSkillRepository;
use super::super::{HostedSkillStore, NewSkill, SkillPage, StoreError};
use super::support::{self, SKILL_ID, admin, principal};

type Store<'db> = HostedSkillStore<PrismaSkillRepository<'db>>;

#[tokio::test]
async fn postgres_owner_roundtrip() {
    let fixture = support::provision().await;
    let writer = support::store(&fixture);
    support::migrate(&writer).await;
    let availability = Availability::default();
    let store = managed_store(&writer, &availability);
    let created = create_row(&store).await;
    assert_created_fields(&created);
    assert_stored_columns(&writer).await;
    assert_owner_visibility(&store).await;
    delete_row(&store).await;
    assert_page_order(&store, &writer).await;
    drop(store);
    writer.pool.unwrap().close();
    fixture.close_async().await.unwrap();
}

fn managed_store<'db>(writer: &'db PostgresStore, availability: &'db Availability) -> Store<'db> {
    let client = PrismaClient {
        writer,
        reader: writer,
        availability,
    };
    HostedSkillStore::new(
        PrismaSkillRepository::new(Some(client), support::no_publish()),
        support::fixed_ids(),
        support::fixed_clock(),
    )
    .unwrap()
}

async fn create_row(store: &Store<'_>) -> SkillTable {
    let mut metadata = BTreeMap::new();
    metadata.insert("note".to_owned(), JsonData::String("a\0b".to_owned()));
    metadata.insert(
        "tier".to_owned(),
        JsonData::Number(serde_json::Number::from(2_i32)),
    );
    let data = NewSkill {
        display_title: Some("Round trip".to_owned()),
        description: Some("Stored".to_owned()),
        instructions: None,
        metadata: Some(metadata),
        file_content: Some(b"zip".to_vec()),
        file_name: Some("skill.zip".to_owned()),
        file_type: Some("application/zip".to_owned()),
    };
    store
        .create_skill(&data, None, Some(&principal("alice")))
        .await
        .unwrap()
}

fn assert_created_fields(created: &SkillTable) {
    assert_eq!(created.skill_id, SKILL_ID);
    assert_eq!(created.source, "custom");
    assert_eq!(created.created_by.as_deref(), Some("alice"));
    assert_eq!(created.updated_by.as_deref(), Some("alice"));
    assert_eq!(created.file_content.as_deref(), Some("emlw"));
    assert_eq!(created.file_name.as_deref(), Some("skill.zip"));
    assert_eq!(
        created
            .metadata
            .as_ref()
            .map(|metadata| metadata.get("tier")),
        Some(Some(&JsonData::Number(serde_json::Number::from(2_i32))))
    );
    assert!(created.created_at.is_some());
    assert!(created.updated_at.is_some());
}

async fn assert_stored_columns(writer: &PostgresStore) {
    let raw = writer
        .session()
        .await
        .unwrap()
        .query_one(
            "SELECT file_content, metadata::text FROM \"LiteLLM_SkillsTable\" WHERE skill_id = $1",
            &[&SKILL_ID],
        )
        .await
        .unwrap();
    let bytes: Option<Vec<u8>> = raw.try_get(0).unwrap();
    assert_eq!(bytes, Some(b"zip".to_vec()));
    let stored_metadata: String = raw.try_get(1).unwrap();
    assert!(stored_metadata.contains("\"note\": \"ab\""));
}

async fn assert_owner_visibility(store: &Store<'_>) {
    assert_listed_for(store, Some(&principal("alice")), 1).await;
    assert_listed_for(store, Some(&principal("bob")), 0).await;
    assert_listed_for(store, Some(&admin("root")), 1).await;
    assert_listed_for(store, None, 1).await;
    assert!(matches!(
        store
            .get_skill(SKILL_ID, Some(&principal("bob")))
            .await
            .unwrap_err(),
        StoreError::NotFound { .. }
    ));
    assert_eq!(
        store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap()
            .skill_id,
        SKILL_ID
    );
}

async fn assert_listed_for(store: &Store<'_>, caller: Option<&Principal>, expected: usize) {
    let rows = store
        .list_skills(SkillPage::default(), caller)
        .await
        .unwrap();
    assert_eq!(rows.len(), expected);
    if let Some(row) = rows.first() {
        assert_eq!(row.skill_id, SKILL_ID);
    }
}

async fn delete_row(store: &Store<'_>) {
    let deleted = store
        .delete_skill(SKILL_ID, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(deleted.id, SKILL_ID);
    assert_eq!(deleted.r#type, "skill_deleted");
    let page = SkillPage::default();
    assert!(
        store
            .list_skills(page, Some(&principal("alice")))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        store
            .get_skill(SKILL_ID, Some(&principal("alice")))
            .await
            .unwrap_err(),
        StoreError::NotFound { .. }
    ));
}

async fn insert_row(writer: &PostgresStore, skill_id: &str, created_at: &str) {
    let title = "Older";
    let owner = "alice";
    writer
        .session()
        .await
        .unwrap()
        .execute(
            "INSERT INTO \"LiteLLM_SkillsTable\" (skill_id, display_title, created_by, created_at) \
             VALUES ($1, $2, $3, $4::text::timestamp(3))",
            &[&skill_id, &title, &owner, &created_at],
        )
        .await
        .unwrap();
}

async fn assert_page_order(store: &Store<'_>, writer: &PostgresStore) {
    let older = "litellm_skill_older";
    let oldest = "litellm_skill_oldest";
    insert_row(writer, older, "2020-01-01 00:00:00").await;
    insert_row(writer, oldest, "2019-01-01 00:00:00").await;
    let rows = store
        .list_skills(SkillPage::default(), Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].skill_id, older);
    assert_eq!(rows[1].skill_id, oldest);
    let next_page = store
        .list_skills(
            SkillPage {
                limit: 1,
                offset: 1,
            },
            Some(&principal("alice")),
        )
        .await
        .unwrap();
    assert_eq!(next_page.len(), 1);
    assert_eq!(next_page[0].skill_id, oldest);
}
