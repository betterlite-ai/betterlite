use std::collections::BTreeMap;

use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use deadpool_postgres::GenericClient as _;

use super::super::repository::PrismaSkillRepository;
use super::super::{HostedSkillStore, NewSkill, SkillPage, StoreError};
use super::support::{self, SKILL_ID, admin, principal};

#[tokio::test]
async fn postgres_owner_roundtrip() {
    let fixture = support::provision().await;
    let writer = support::store(&fixture);
    support::migrate(&writer).await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &writer,
        reader: &writer,
        availability: &availability,
    };
    let store = HostedSkillStore::new(
        PrismaSkillRepository::new(Some(client), support::no_publish()),
        support::fixed_ids(),
        support::fixed_clock(),
    )
    .unwrap();

    let mut metadata = BTreeMap::new();
    metadata.insert("note".to_owned(), JsonData::String("a\0b".to_owned()));
    metadata.insert(
        "tier".to_owned(),
        JsonData::Number(serde_json::Number::from(2)),
    );
    let data = NewSkill {
        display_title: Some("Round trip".to_owned()),
        description: Some("Stored".to_owned()),
        instructions: None,
        metadata: Some(metadata.clone()),
        file_content: Some(b"zip".to_vec()),
        file_name: Some("skill.zip".to_owned()),
        file_type: Some("application/zip".to_owned()),
    };
    let created = store
        .create_skill(&data, None, Some(&principal("alice")))
        .await
        .unwrap();
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
        Some(Some(&JsonData::Number(serde_json::Number::from(2))))
    );
    assert!(created.created_at.is_some());
    assert!(created.updated_at.is_some());

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

    let page = SkillPage::default();
    let mine = store
        .list_skills(page, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].skill_id, SKILL_ID);
    assert!(
        store
            .list_skills(page, Some(&principal("bob")))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store
            .list_skills(page, Some(&admin("root")))
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.list_skills(page, None).await.unwrap().len(), 1);

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
            .unwrap(),
        created
    );

    let deleted = store
        .delete_skill(SKILL_ID, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(deleted.id, SKILL_ID);
    assert_eq!(deleted.r#type, "skill_deleted");
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

    let older = "litellm_skill_older";
    let older_title = "Older";
    let older_owner = "alice";
    writer
        .session()
        .await
        .unwrap()
        .execute(
            "INSERT INTO \"LiteLLM_SkillsTable\" (skill_id, display_title, created_by, created_at) \
             VALUES ($1, $2, $3, TIMESTAMP '2020-01-01 00:00:00')",
            &[&older, &older_title, &older_owner],
        )
        .await
        .unwrap();
    let ordered = store
        .list_skills(page, Some(&principal("alice")))
        .await
        .unwrap();
    assert_eq!(ordered.len(), 2);
    assert_eq!(ordered[0].skill_id, SKILL_ID);
    assert_eq!(ordered[1].skill_id, older);
    let second = store
        .list_skills(
            SkillPage {
                limit: 1,
                offset: 1,
            },
            Some(&principal("alice")),
        )
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].skill_id, older);

    drop(store);
    writer.pool.unwrap().close();
    fixture.close_async().await.unwrap();
}
