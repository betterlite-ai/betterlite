use betterlite_openai::batches::records::{BatchObject, BatchStatus};

use crate::budget_permission_rows::JsonData;

use super::super::batch::ManagedBatchRepository;
use super::super::decoder::decode_managed_batch_record;
use super::support::{Case, Journal, insert_batch, recording_publisher};
use super::{BATCH_JSON, unchanged, write_of};

struct AuthorRow {
    file_object: String,
    status: String,
    updated_by: Option<String>,
    created_by: Option<String>,
    team_id: Option<String>,
    api_key: Option<String>,
    model_object_id: String,
    request_tags: Option<String>,
}

async fn seed_author_row(case: &Case) -> String {
    insert_batch(&case.writer, "author-null", Some("in_progress"), BATCH_JSON).await;
    let client = case.writer.session().await.unwrap();
    client
        .execute(
            "UPDATE \"LiteLLM_ManagedObjectTable\" SET created_by = 'creator', team_id = 'team', api_key = 'key', request_tags = '[\"tag\"]'::jsonb WHERE unified_object_id = 'author-null'",
            &[],
        )
        .await
        .unwrap();
    let model_object_id = client
        .query_one(
            "SELECT model_object_id FROM \"LiteLLM_ManagedObjectTable\" WHERE unified_object_id = 'author-null'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    drop(client);
    model_object_id
}

async fn cas_author_row_to_completed(case: &Case) -> BatchObject {
    let repository = ManagedBatchRepository::new(
        Some(case.client()),
        recording_publisher(&Journal::default()),
    );
    let mut stored = repository
        .load_batch::<BatchObject>("author-null")
        .await
        .unwrap()
        .unwrap();
    stored.id = "author-null".to_owned();
    stored.status = BatchStatus::Completed;
    let written = repository
        .compare_and_set(
            &write_of(&stored, "completed"),
            &unchanged(Some("in_progress")),
            None,
        )
        .await
        .unwrap();
    assert!(written);
    stored
}

async fn author_row(case: &Case) -> AuthorRow {
    let client = case.writer.session().await.unwrap();
    let row = client
        .query_one(
            "SELECT file_object::text, status, updated_by, created_by, team_id, api_key, model_object_id, request_tags::text FROM \"LiteLLM_ManagedObjectTable\" WHERE unified_object_id = 'author-null'",
            &[],
        )
        .await
        .unwrap();
    let fields = AuthorRow {
        file_object: row.try_get(0).unwrap(),
        status: row.try_get(1).unwrap(),
        updated_by: row.try_get(2).unwrap(),
        created_by: row.try_get(3).unwrap(),
        team_id: row.try_get(4).unwrap(),
        api_key: row.try_get(5).unwrap(),
        model_object_id: row.try_get(6).unwrap(),
        request_tags: row.try_get(7).unwrap(),
    };
    drop(client);
    fields
}

#[tokio::test]
async fn cas_only_three_fields_including_null_author() {
    let case = Case::new().await;
    let model_object_id = seed_author_row(&case).await;
    let stored = cas_author_row_to_completed(&case).await;
    let row = author_row(&case).await;
    // Prisma stores a Python str in a Json field as a JSON string, not as a decoded object
    assert!(matches!(
        serde_json::from_str::<JsonData>(&row.file_object).unwrap(),
        JsonData::String(_)
    ));
    let written = decode_managed_batch_record::<BatchObject>(row.file_object.as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(written, stored);
    assert_eq!(row.status, "completed");
    assert!(row.updated_by.is_none());
    assert_eq!(row.created_by.as_deref(), Some("creator"));
    assert_eq!(row.team_id.as_deref(), Some("team"));
    assert_eq!(row.api_key.as_deref(), Some("key"));
    assert_eq!(row.model_object_id, model_object_id);
    assert_eq!(row.request_tags.as_deref(), Some("[\"tag\"]"));
    case.close().await;
}
