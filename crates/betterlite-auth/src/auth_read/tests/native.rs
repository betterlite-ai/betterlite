#![cfg(test)]

use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::postgres_store::rows::{
    FindMany as RowsFindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use betterlite_db::user_repository::tables::USER_TABLE;
use tokio_postgres::Row;

use crate::auth_read::include::{Including, IncludingQuery, SelectingTable, UserInclude};
use crate::auth_read::repository::RepositoryTable;
use crate::auth_read::table::{AuthTable, Create, FindMany, FindOne, Presence};

use super::support::{assert_connect, native_key};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Observed {
    Key {
        memberships: bool,
    },
    List {
        filter: Vec<String>,
        take: Option<i64>,
        memberships: bool,
    },
    Insert {
        memberships: bool,
    },
}

#[derive(Clone)]
struct RecordingTable {
    observed: Arc<Mutex<Vec<Observed>>>,
}

impl RecordingTable {
    fn record(&self, call: Observed) {
        self.observed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

fn filtered(options: &RowsFindMany<'_, String, Infallible>) -> Vec<String> {
    match options.filter {
        Some(values) => values.to_vec(),
        None => Vec::new(),
    }
}

impl RowCodec for RecordingTable {
    type Model = ();
    type Error = Infallible;

    fn decode(&self, _row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        Ok(None)
    }
}

impl Table for RecordingTable {
    type Column = Infallible;
    type Filter = String;
    type Order = Infallible;
    type Data = ();

    fn keyed<'query>(
        &'query self,
        _action: KeyAction<'query, ()>,
        _key: Key<'query, Infallible>,
    ) -> Query<'query> {
        self.record(Observed::Key { memberships: false });
        Query {
            sql: "keyed",
            parameters: Vec::new(),
        }
    }

    fn list<'query>(
        &'query self,
        options: RowsFindMany<'query, String, Infallible>,
    ) -> Query<'query> {
        self.record(Observed::List {
            filter: filtered(&options),
            take: options.take,
            memberships: false,
        });
        Query {
            sql: "list",
            parameters: Vec::new(),
        }
    }

    fn insert<'query>(&'query self, _data: &'query ()) -> Query<'query> {
        self.record(Observed::Insert { memberships: false });
        Query {
            sql: "insert",
            parameters: Vec::new(),
        }
    }

    fn count<'query>(&'query self, _filter: Option<&'query [String]>) -> Query<'query> {
        Query {
            sql: "count",
            parameters: Vec::new(),
        }
    }

    fn membership(&self, _column: KeyColumn<Infallible>) -> &str {
        "membership"
    }
}

impl Including for RecordingTable {
    type Include = UserInclude;

    fn including<'query>(
        &'query self,
        query: IncludingQuery<'query, Self>,
        include: Option<&'query Self::Include>,
    ) -> Query<'query> {
        let memberships = include.is_some_and(|include| include.organization_memberships);
        match query {
            IncludingQuery::Keyed { .. } => {
                self.record(Observed::Key { memberships });
                Query {
                    sql: "keyed_including",
                    parameters: Vec::new(),
                }
            }
            IncludingQuery::Listed { options } => {
                self.record(Observed::List {
                    filter: filtered(&options),
                    take: options.take,
                    memberships,
                });
                Query {
                    sql: "list_including",
                    parameters: Vec::new(),
                }
            }
            IncludingQuery::Inserted { .. } => {
                self.record(Observed::Insert { memberships });
                Query {
                    sql: "insert_including",
                    parameters: Vec::new(),
                }
            }
        }
    }
}

#[tokio::test]
async fn native_adapter_forwards_relations_and_first_filter() {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let repository: RepositoryTable<deadpool_postgres::Client, RecordingTable> = RepositoryTable {
        client: None,
        table: RecordingTable {
            observed: Arc::clone(&observed),
        },
    };
    let include = Presence::Value(UserInclude {
        organization_memberships: true,
    });
    let filter = vec![
        "user_email ILIKE $1".to_owned(),
        "deleted_at IS NULL".to_owned(),
    ];
    assert_connect(
        repository
            .find_unique(FindOne {
                filter: native_key(),
                include: include.clone(),
            })
            .await,
    );
    assert_connect(
        repository
            .find_first(FindOne {
                filter: filter.clone(),
                include: include.clone(),
            })
            .await,
    );
    assert_connect(
        repository
            .find_many(FindMany {
                filter: Presence::Omitted,
                include: Presence::Omitted,
                take: Presence::Value(7),
            })
            .await,
    );
    assert_connect(
        repository
            .create(Create {
                data: (),
                include: include.clone(),
            })
            .await,
    );
    let calls = observed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        calls,
        vec![
            Observed::Key { memberships: true },
            Observed::List {
                filter,
                take: Some(1),
                memberships: true,
            },
            Observed::List {
                filter: Vec::new(),
                take: Some(7),
                memberships: false,
            },
            Observed::Insert { memberships: true },
        ]
    );
}

#[test]
fn native_user_include_selects_membership_relations() {
    let user_id = "user-1";
    let selected = SelectingTable {
        table: USER_TABLE,
        include: Some(UserInclude {
            organization_memberships: true,
        }),
    };
    let by_id = selected.keyed(
        KeyAction::Find,
        Key {
            column: KeyColumn::Id,
            value: &user_id,
        },
    );
    assert!(by_id.sql.contains("\"LiteLLM_OrganizationMembership\""));
    assert!(by_id.sql.contains("'organization_memberships'"));
    assert_eq!(by_id.parameters.len(), 1);
    let listed = selected.list(RowsFindMany {
        filter: None,
        order: None,
        skip: None,
        take: Some(1),
    });
    assert!(listed.sql.contains("\"LiteLLM_OrganizationMembership\""));
    let created = selected.insert(&());
    assert!(created.sql.starts_with("INSERT INTO \"LiteLLM_UserTable\""));
    assert!(created.sql.contains("\"LiteLLM_OrganizationMembership\""));
    let plain = SelectingTable {
        table: USER_TABLE,
        include: None,
    };
    let base_table = USER_TABLE;
    let base = Table::keyed(
        &base_table,
        KeyAction::Find,
        Key {
            column: KeyColumn::Id,
            value: &user_id,
        },
    );
    let plain_by_id = plain.keyed(
        KeyAction::Find,
        Key {
            column: KeyColumn::Id,
            value: &user_id,
        },
    );
    assert_eq!(
        plain_by_id.sql, base.sql,
        "no selection keeps B700's by-id statement"
    );
    let unselected = SelectingTable {
        table: USER_TABLE,
        include: Some(UserInclude {
            organization_memberships: false,
        }),
    };
    let unselected_by_id = unselected.keyed(
        KeyAction::Find,
        Key {
            column: KeyColumn::Id,
            value: &user_id,
        },
    );
    assert_eq!(
        unselected_by_id.sql, base.sql,
        "an unrequested relation keeps B700's by-id statement"
    );
}
