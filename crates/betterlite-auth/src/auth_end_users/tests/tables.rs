#![cfg(test)]

use std::error::Error;
use std::fmt::Debug;
use std::sync::Arc;

use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;

use super::super::postgres::BudgetTable;
use super::super::rows::{EndUserRowError, EndUserRowTable};
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

// the repository fault of a call the lookup never makes, which must be the table's own refusal
fn repository_fault<T, Failure>(
    result: Result<T, AuthReadError<Failure>>,
    why: &str,
) -> DbError<Failure>
where
    T: Debug,
    Failure: Error + 'static,
{
    let error = result.expect_err(why);
    let AuthReadError::Repository(fault) = error else {
        panic!("{why}");
    };
    fault
}

async fn budget_table_unread_errors(table: &BudgetTable) -> Vec<DbError<TableError>> {
    vec![
        repository_fault(
            table
                .find_first(FindOne {
                    filter: "budget-1".to_owned(),
                    include: Presence::Omitted,
                })
                .await,
            "find_first is not part of the default-budget lookup",
        ),
        repository_fault(
            table
                .find_many(FindMany {
                    filter: Presence::Value("budget-1".to_owned()),
                    include: Presence::Omitted,
                    take: Presence::Omitted,
                })
                .await,
            "find_many is not part of the default-budget lookup",
        ),
        repository_fault(
            table
                .update(Update {
                    filter: "budget-1".to_owned(),
                    data: (),
                })
                .await,
            "update is not part of the default-budget lookup",
        ),
        repository_fault(
            table
                .create(Create {
                    data: (),
                    include: Presence::Omitted,
                })
                .await,
            "create is not part of the default-budget lookup",
        ),
    ]
}

// the default-budget read and the end-user lookup only ever ask for the unique row
#[tokio::test]
async fn budget_table_reads_only_the_unique_row() {
    let table = budget_table();
    for error in budget_table_unread_errors(&table).await {
        assert!(matches!(error, DbError::CreateNull));
    }
}

async fn end_user_unique_errors(table: &EndUserRowTable) -> Vec<DbError<EndUserRowError>> {
    vec![
        repository_fault(
            table
                .find_unique(FindOne {
                    filter: "alice".to_owned(),
                    include: Presence::Omitted,
                })
                .await,
            "an absent pool cannot answer the row read",
        ),
        repository_fault(
            table
                .find_first(FindOne {
                    filter: "alice".to_owned(),
                    include: Presence::Omitted,
                })
                .await,
            "an absent pool cannot answer the filtered read",
        ),
    ]
}

async fn end_user_unread_errors(table: &EndUserRowTable) -> Vec<DbError<EndUserRowError>> {
    vec![
        repository_fault(
            table
                .find_many(FindMany {
                    filter: Presence::Value("alice".to_owned()),
                    include: Presence::Omitted,
                    take: Presence::Omitted,
                })
                .await,
            "find_many is not part of the end-user lookup",
        ),
        repository_fault(
            table
                .update(Update {
                    filter: "alice".to_owned(),
                    data: (),
                })
                .await,
            "update is not part of the end-user lookup",
        ),
        repository_fault(
            table
                .create(Create {
                    data: (),
                    include: Presence::Omitted,
                })
                .await,
            "create is not part of the end-user lookup",
        ),
    ]
}

#[tokio::test]
async fn end_user_table_reads_only_the_unique_row() {
    let table = EndUserRowTable { client: None };
    for error in end_user_unique_errors(&table).await {
        assert!(matches!(error, DbError::NoDbConnected));
    }
    for error in end_user_unread_errors(&table).await {
        assert!(matches!(error, DbError::CreateNull));
    }
}
