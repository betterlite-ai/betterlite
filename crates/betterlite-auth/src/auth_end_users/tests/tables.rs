#![cfg(test)]

use std::error::Error;
use std::fmt::Debug;
use std::sync::Arc;

use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::session::PostgresStore;
use rstest::rstest;

use super::super::postgres::BudgetTable;
use super::super::rows::EndUserRowTable;
use crate::auth_read::table::{
    AuthReadError, AuthTable, Create, FindMany, FindOne, Presence, Update,
};

fn budget_table() -> BudgetTable {
    BudgetTable {
        writer: Arc::new(PostgresStore { pool: None }),
        reader: Arc::new(PostgresStore { pool: None }),
        availability: Arc::new(Availability::default()),
    }
}

// the repository fault of a call the lookup must not make, which is the table's own refusal
fn repository_fault<Row, Failure>(result: Result<Row, AuthReadError<Failure>>) -> DbError<Failure>
where
    Row: Debug,
    Failure: Error + 'static,
{
    let error = result.expect_err("only the unique row is part of the lookup");
    let AuthReadError::Repository(fault) = error else {
        panic!("the table answered outside its repository");
    };
    fault
}

fn unique_request() -> FindOne<String, ()> {
    FindOne {
        filter: "row-1".to_owned(),
        include: Presence::Omitted,
    }
}

fn many_request() -> FindMany<String, ()> {
    FindMany {
        filter: Presence::Value("row-1".to_owned()),
        include: Presence::Omitted,
        take: Presence::Omitted,
    }
}

fn update_request() -> Update<String, ()> {
    Update {
        filter: "row-1".to_owned(),
        data: (),
    }
}

fn create_request() -> Create<(), ()> {
    Create {
        data: (),
        include: Presence::Omitted,
    }
}

#[derive(Clone, Copy, Debug)]
enum Action {
    FindUnique,
    FindFirst,
    FindMany,
    Update,
    Create,
}

async fn fault<T>(table: &T, action: Action) -> DbError<T::Failure>
where
    T: AuthTable<Key = String, Filter = String, Include = (), Data = ()>,
    T::Row: Debug,
{
    match action {
        Action::FindUnique => repository_fault(table.find_unique(unique_request()).await),
        Action::FindFirst => repository_fault(table.find_first(unique_request()).await),
        Action::FindMany => repository_fault(table.find_many(many_request()).await),
        Action::Update => repository_fault(table.update(update_request()).await),
        Action::Create => repository_fault(table.create(create_request()).await),
    }
}

// the default-budget read only ever asks for the unique row
#[rstest]
#[case::find_first(Action::FindFirst)]
#[case::find_many(Action::FindMany)]
#[case::update(Action::Update)]
#[case::create(Action::Create)]
#[tokio::test]
async fn budget_table_reads_only_the_unique_row(#[case] action: Action) {
    let error = fault(&budget_table(), action).await;
    assert!(matches!(error, DbError::CreateNull));
}

// the end-user lookup's own reads reach the database and fail only because none is connected
#[rstest]
#[case::find_unique(Action::FindUnique)]
#[case::find_first(Action::FindFirst)]
#[tokio::test]
async fn end_user_table_reads_only_the_unique_row(#[case] action: Action) {
    let error = fault(&EndUserRowTable { client: None }, action).await;
    assert!(matches!(error, DbError::NoDbConnected));
}

#[rstest]
#[case::find_many(Action::FindMany)]
#[case::update(Action::Update)]
#[case::create(Action::Create)]
#[tokio::test]
async fn end_user_table_refuses_the_unread_actions(#[case] action: Action) {
    let error = fault(&EndUserRowTable { client: None }, action).await;
    assert!(matches!(error, DbError::CreateNull));
}
