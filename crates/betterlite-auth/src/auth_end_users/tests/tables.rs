#![cfg(test)]

use std::sync::Arc;

use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;

use super::super::postgres::BudgetTable;
use super::super::rows::EndUserRowTable;
use crate::auth_read::table::{
    AuthReadError, AuthTable as _, Create, FindMany, FindOne, Presence, Update,
};

fn budget_table() -> BudgetTable {
    BudgetTable {
        writer: Arc::new(PostgresStore { pool: None }),
        reader: Arc::new(PostgresStore { pool: None }),
        availability: Arc::new(Availability::default()),
    }
}

// the default-budget read and the end-user lookup only ever ask for the unique row
#[tokio::test]
async fn budget_table_reads_only_the_unique_row() {
    let table = budget_table();
    let first = table
        .find_first(FindOne {
            filter: "budget-1".to_owned(),
            include: Presence::Omitted,
        })
        .await
        .expect_err("find_first is not part of the default-budget lookup");
    assert!(matches!(
        first,
        AuthReadError::Repository(DbError::CreateNull)
    ));
    let many = table
        .find_many(FindMany {
            filter: Presence::Value("budget-1".to_owned()),
            include: Presence::Omitted,
            take: Presence::Omitted,
        })
        .await
        .expect_err("find_many is not part of the default-budget lookup");
    assert!(matches!(
        many,
        AuthReadError::Repository(DbError::CreateNull)
    ));
    let updated = table
        .update(Update {
            filter: "budget-1".to_owned(),
            data: (),
        })
        .await
        .expect_err("update is not part of the default-budget lookup");
    assert!(matches!(
        updated,
        AuthReadError::Repository(DbError::CreateNull)
    ));
    let created = table
        .create(Create {
            data: (),
            include: Presence::Omitted,
        })
        .await
        .expect_err("create is not part of the default-budget lookup");
    assert!(matches!(
        created,
        AuthReadError::Repository(DbError::CreateNull)
    ));
}

#[tokio::test]
async fn end_user_table_reads_only_the_unique_row() {
    let table = EndUserRowTable { client: None };
    let unique = table
        .find_unique(FindOne {
            filter: "alice".to_owned(),
            include: Presence::Omitted,
        })
        .await
        .expect_err("an absent pool cannot answer the row read");
    assert!(matches!(
        unique,
        AuthReadError::Repository(DbError::NoDbConnected)
    ));
    let first = table
        .find_first(FindOne {
            filter: "alice".to_owned(),
            include: Presence::Omitted,
        })
        .await
        .expect_err("an absent pool cannot answer the filtered read");
    assert!(matches!(
        first,
        AuthReadError::Repository(DbError::NoDbConnected)
    ));
    let many = table
        .find_many(FindMany {
            filter: Presence::Value("alice".to_owned()),
            include: Presence::Omitted,
            take: Presence::Omitted,
        })
        .await
        .expect_err("find_many is not part of the end-user lookup");
    assert!(matches!(
        many,
        AuthReadError::Repository(DbError::CreateNull)
    ));
    let updated = table
        .update(Update {
            filter: "alice".to_owned(),
            data: (),
        })
        .await
        .expect_err("update is not part of the end-user lookup");
    assert!(matches!(
        updated,
        AuthReadError::Repository(DbError::CreateNull)
    ));
    let created = table
        .create(Create {
            data: (),
            include: Presence::Omitted,
        })
        .await
        .expect_err("create is not part of the end-user lookup");
    assert!(matches!(
        created,
        AuthReadError::Repository(DbError::CreateNull)
    ));
}
