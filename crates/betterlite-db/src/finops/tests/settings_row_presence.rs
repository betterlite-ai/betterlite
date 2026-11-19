use crate::budget_permission_rows::JsonData;
use crate::finops::settings_row_presence::settings_row_present;

use super::support::Database;
use super::support::json_object;
use super::support::seed_null;
use super::support::seed_string;

const PARAM: &str = "settings_row_under_test";

#[tokio::test]
async fn settings_row_present_counts_a_row_that_holds_a_value() {
    let database = Database::provision().await;
    assert!(
        !settings_row_present(Some(database.client()), PARAM)
            .await
            .unwrap()
    );
    seed_string(
        &database,
        PARAM,
        &json_object(&[("timezone", JsonData::String("UTC".to_owned()))]),
    )
    .await;
    assert!(
        settings_row_present(Some(database.client()), PARAM)
            .await
            .unwrap()
    );
    database.close().await;
}

#[tokio::test]
async fn settings_row_present_ignores_a_null_value() {
    let database = Database::provision().await;
    seed_null(&database, PARAM).await;
    assert!(
        !settings_row_present(Some(database.client()), PARAM)
            .await
            .unwrap()
    );
    database.close().await;
}

#[tokio::test]
async fn settings_row_present_counts_a_value_that_does_not_decode() {
    let database = Database::provision().await;
    seed_string(&database, PARAM, "not json").await;
    assert!(
        settings_row_present(Some(database.client()), PARAM)
            .await
            .unwrap()
    );
    database.close().await;
}
