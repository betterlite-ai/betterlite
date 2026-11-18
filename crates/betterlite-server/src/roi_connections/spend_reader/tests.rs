#![cfg(test)]

use std::sync::Arc;

use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::NaiveDate;

use crate::roi_connections::spend_reader::spend_reader;
use crate::roi_settings::repository::DatabaseSettingsRepository;
use crate::roi_spend::SpendReader as _;

#[tokio::test]
async fn spend_reader_reads_through_the_repository_store() {
    let repository = DatabaseSettingsRepository {
        store: Arc::new(PostgresStore { pool: None }),
        availability: Availability::default(),
    };
    let reader = spend_reader(&repository);
    let start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("start date");
    let end = NaiveDate::from_ymd_opt(2026, 1, 31).expect("end date");
    let error = reader.read(start, end).await.err().expect("no database");
    assert!(matches!(error, DbError::NoDbConnected));
}
