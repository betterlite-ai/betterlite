use betterlite_redis::config_sync::table_actions::TableActions as _;

use super::super::bindings::UISettings;
use super::super::repository::PrismaTableRepository;
use super::super::user_banner::{BannerActions, UiSettingsRow};
use super::support::{Case, Journal, recording_publisher};

#[tokio::test]
async fn banner_create_many_and_delete_many_count_rows() {
    let case = Case::new().await;
    let journal = Journal::default();
    let base = PrismaTableRepository::<UISettings>::new(
        Some(case.client()),
        recording_publisher(&journal),
        UISettings,
    );
    let table = base.table(|access| BannerActions { access }).unwrap();
    let rows = vec![
        UiSettingsRow {
            id: "many-one".to_owned(),
            ui_settings: "one".to_owned(),
        },
        UiSettingsRow {
            id: "many-two".to_owned(),
            ui_settings: "two".to_owned(),
        },
    ];
    assert_eq!(table.create_many(rows).await.unwrap(), 2);
    assert_eq!(table.delete_many("many-one".to_owned()).await.unwrap(), 1);
    assert_eq!(table.delete_many("missing-id".to_owned()).await.unwrap(), 0);
    assert_eq!(journal.entries().len(), 3);
    let client = case.writer.session().await.unwrap();
    let stored = client
        .query(
            "SELECT id, ui_settings::text FROM \"LiteLLM_UISettings\" ORDER BY id",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].try_get::<_, String>(0).unwrap(), "many-two");
    assert_eq!(stored[0].try_get::<_, String>(1).unwrap(), "\"two\"");
    drop(client);
    case.close().await;
}

#[tokio::test]
async fn banner_create_many_rolls_back_a_partial_batch() {
    let case = Case::new().await;
    let journal = Journal::default();
    let base = PrismaTableRepository::<UISettings>::new(
        Some(case.client()),
        recording_publisher(&journal),
        UISettings,
    );
    let table = base.table(|access| BannerActions { access }).unwrap();
    let rows = vec![
        UiSettingsRow {
            id: "rollback-first".to_owned(),
            ui_settings: "first".to_owned(),
        },
        UiSettingsRow {
            id: "rollback-first".to_owned(),
            ui_settings: "again".to_owned(),
        },
    ];
    assert!(table.create_many(rows).await.is_err());
    assert!(journal.entries().is_empty());
    let client = case.writer.session().await.unwrap();
    let stored = client
        .query(
            "SELECT count(*) FROM \"LiteLLM_UISettings\" WHERE id = 'rollback-first'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(stored[0].try_get::<_, i64>(0).unwrap(), 0);
    drop(client);
    case.close().await;
}
