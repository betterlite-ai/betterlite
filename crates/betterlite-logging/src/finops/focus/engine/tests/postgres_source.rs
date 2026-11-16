use betterlite_db::finops::focus::FocusUsageError;
use betterlite_db::postgres_store::session::PostgresStore;

use crate::finops::focus::engine::source::{
    PostgresUsageSource, UsageSource as _, UsageSourceError,
};

#[tokio::test]
async fn usage_data_reports_a_store_without_a_database() {
    let source = PostgresUsageSource {
        store: PostgresStore { pool: None },
    };
    let error = source
        .usage_data(Some(10), None, None)
        .await
        .expect_err("the unconnected store refuses the query");
    assert!(matches!(
        error,
        UsageSourceError::Query(FocusUsageError::Database { .. })
    ));
}

#[tokio::test]
async fn usage_data_passes_a_negative_limit_to_the_query() {
    let source = PostgresUsageSource {
        store: PostgresStore { pool: None },
    };
    let error = source
        .usage_data(Some(-1), None, None)
        .await
        .expect_err("the negative limit is refused");
    assert!(matches!(
        error,
        UsageSourceError::Query(FocusUsageError::NegativeLimit)
    ));
}
