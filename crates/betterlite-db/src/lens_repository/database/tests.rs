#![cfg(test)]

use deadpool_postgres::Client;

use super::{Database, DatabaseError, WriterDatabase};
use crate::daily_spend_sql::bind::SqlBind;
use crate::lens_repository::support::database;

#[test]
fn writer_database_implements_the_database_protocol() {
    fn accepts<Db>()
    where
        Db: Database,
    {
    }

    accepts::<WriterDatabase<Client>>();
}

#[tokio::test]
async fn writer_database_returns_the_text_data_of_each_row() {
    let db = database().await;
    let writer = WriterDatabase {
        writer: db.pool.get().await.unwrap(),
    };
    let rows = writer
        .query_raw(r#"SELECT '{"id":"lens-a"}'::jsonb::text AS data"#, &[])
        .await
        .unwrap();
    assert_eq!(rows.first().unwrap().data, r#"{"id": "lens-a"}"#);
    db.close().await;
}

#[tokio::test]
async fn writer_database_rejects_a_row_whose_data_is_not_text() {
    let db = database().await;
    let writer = WriterDatabase {
        writer: db.pool.get().await.unwrap(),
    };
    let result = writer.query_raw("SELECT 1 AS data", &[]).await;
    assert!(matches!(result, Err(DatabaseError::Row(_))));
    db.close().await;
}

#[tokio::test]
async fn writer_database_rejects_a_row_without_a_data_column() {
    let db = database().await;
    let writer = WriterDatabase {
        writer: db.pool.get().await.unwrap(),
    };
    let result = writer.query_raw("SELECT 1 AS value", &[]).await;
    assert!(matches!(result, Err(DatabaseError::Row(_))));
    db.close().await;
}

#[tokio::test]
async fn writer_database_binds_text_and_integer_arguments() {
    let db = database().await;
    let writer = WriterDatabase {
        writer: db.pool.get().await.unwrap(),
    };
    let rows = writer
        .query_raw(
            "SELECT ($1::text || ':' || $2::bigint)::text AS data",
            &[
                SqlBind::Text(Some("answer".to_owned())),
                SqlBind::Integer(Some(42)),
            ],
        )
        .await
        .unwrap();
    assert_eq!(rows.first().unwrap().data, "answer:42");
    db.close().await;
}

#[tokio::test]
async fn writer_database_returns_the_affected_row_count() {
    let db = database().await;
    let writer = WriterDatabase {
        writer: db.pool.get().await.unwrap(),
    };
    writer
        .execute_raw("CREATE TABLE probe_rows (id TEXT)", &[])
        .await
        .unwrap();
    let inserted = writer
        .execute_raw(
            "INSERT INTO probe_rows (id) VALUES ($1), ($2)",
            &[
                SqlBind::Text(Some("a".to_owned())),
                SqlBind::Text(Some("b".to_owned())),
            ],
        )
        .await
        .unwrap();
    let deleted = writer
        .execute_raw(
            "DELETE FROM probe_rows WHERE id=$1",
            &[SqlBind::Text(Some("a".to_owned()))],
        )
        .await
        .unwrap();
    assert_eq!((inserted, deleted), (2, 1));
    db.close().await;
}
