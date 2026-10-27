#![cfg(test)]

use super::errors::DbError;
use super::session::PostgresStore;

#[tokio::test]
async fn missing_client_exact_message() {
    let store = PostgresStore { pool: None };
    let error = store.session().await.err().unwrap();
    assert!(matches!(error, DbError::NoDbConnected));
    assert_eq!(
        error.to_string(),
        "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
}
