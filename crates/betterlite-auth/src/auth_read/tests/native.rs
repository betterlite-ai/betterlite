#![cfg(test)]

use std::convert::Infallible;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::postgres_store::rows::{
    FindMany as RowsFindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use tokio_postgres::Row;

use crate::auth_read::include::{Including, IncludingQuery, SelectingTable, UserInclude};
use crate::auth_read::jwt_rows::{JwtMappingFilter, JwtRows};
use crate::auth_read::repository::RepositoryTable;
use crate::auth_read::table::{AuthTable, Create, FindMany, FindOne, Presence};
use crate::auth_read::user_rows::{UserCreate, UserEmailFilter, UserRows};

use super::support::{assert_connect, native_key};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Observed {
    Key {
        memberships: bool,
    },
    List {
        filter: Option<String>,
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

fn first_filter(options: &RowsFindMany<'_, String, Infallible>) -> Option<String> {
    options.filter.and_then(|filters| filters.first()).cloned()
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
            filter: first_filter(&options),
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
        let memberships = include.is_some_and(|selected| selected.organization_memberships);
        match query {
            IncludingQuery::Keyed { .. } => {
                self.record(Observed::Key { memberships });
                Query {
                    sql: "keyed_including",
                    parameters: Vec::new(),
                }
            }
            IncludingQuery::Listed { filter, limit } => {
                self.record(Observed::List {
                    filter: filter.cloned(),
                    take: *limit,
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

fn user_create() -> UserCreate {
    UserCreate {
        user_id: "user-3".to_owned(),
        user_alias: Some("third".to_owned()),
        team_id: None,
        sso_user_id: None,
        organization_id: None,
        object_permission_id: None,
        user_role: Some("internal_user".to_owned()),
        user_email: Some("third@example.com".to_owned()),
        teams: Vec::new(),
        models: Vec::new(),
        max_parallel_requests: None,
        tpm_limit: Some(10),
        rpm_limit: None,
        budget_duration: None,
        allowed_cache_controls: Vec::new(),
        policies: Vec::new(),
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
    let filter = "user_email ILIKE $1".to_owned();
    assert_connect(
        repository
            .find_unique(FindOne {
                filter: native_key(),
                include,
            })
            .await,
    );
    assert_connect(
        repository
            .find_first(FindOne {
                filter: filter.clone(),
                include,
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
    assert_connect(repository.create(Create { data: (), include }).await);
    let calls = observed
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(
        calls,
        vec![
            Observed::Key { memberships: true },
            Observed::List {
                filter: Some(filter),
                take: Some(1),
                memberships: true,
            },
            Observed::List {
                filter: None,
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
        table: UserRows,
        include: Some(UserInclude {
            organization_memberships: true,
        }),
        limit: Some(1),
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
    assert!(listed.sql.contains("LIMIT $1"));
    assert_eq!(listed.parameters.len(), 1);
    let data = user_create();
    let created = selected.insert(&data);
    assert!(created.sql.starts_with("WITH inserted AS ("));
    assert!(created.sql.contains("INSERT INTO \"LiteLLM_UserTable\""));
    assert!(created.sql.contains("\"LiteLLM_OrganizationMembership\""));
    assert_eq!(created.parameters.len(), 16);
    let plain = SelectingTable {
        table: UserRows,
        include: None,
        limit: None,
    };
    let plain_by_id = plain.keyed(
        KeyAction::Find,
        Key {
            column: KeyColumn::Id,
            value: &user_id,
        },
    );
    assert!(!plain_by_id.sql.contains("OrganizationMembership"));
    assert_eq!(
        plain
            .list(RowsFindMany {
                filter: None,
                order: None,
                skip: None,
                take: None,
            })
            .sql,
        "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry LIMIT $1",
        "no selection keeps the plain list statement"
    );
    let plain_insert = plain.insert(&data);
    assert!(!plain_insert.sql.contains("OrganizationMembership"));
    assert_eq!(plain_insert.parameters.len(), 16);
    assert_eq!(
        plain.count(None).sql,
        "SELECT count(*) FROM \"LiteLLM_UserTable\""
    );
    assert!(plain.membership(KeyColumn::Id).contains("ANY("));
    let unselected = SelectingTable {
        table: UserRows,
        include: Some(UserInclude {
            organization_memberships: false,
        }),
        limit: None,
    };
    let unselected_by_id = unselected.keyed(
        KeyAction::Find,
        Key {
            column: KeyColumn::Id,
            value: &user_id,
        },
    );
    assert!(!unselected_by_id.sql.contains("OrganizationMembership"));
}

#[test]
fn native_email_filters_choose_the_statement() {
    let table = SelectingTable {
        table: UserRows,
        include: None,
        limit: Some(1),
    };
    let filter = UserEmailFilter {
        address: Some("user@example.com".to_owned()),
        insensitive: true,
    };
    let filters = [filter];
    let insensitive = table.list(RowsFindMany {
        filter: Some(&filters[..]),
        order: None,
        skip: None,
        take: None,
    });
    assert!(
        insensitive
            .sql
            .contains("LOWER(entry.\"user_email\") = LOWER($1)")
    );
    assert!(insensitive.sql.ends_with("LIMIT $2"));
    assert_eq!(insensitive.parameters.len(), 2);
    let exact = [UserEmailFilter {
        address: Some("user@example.com".to_owned()),
        insensitive: false,
    }];
    let exact_match = table.list(RowsFindMany {
        filter: Some(&exact[..]),
        order: None,
        skip: None,
        take: None,
    });
    assert!(exact_match.sql.contains("entry.\"user_email\" = $1"));
    assert!(!exact_match.sql.contains("LOWER"));
    assert_eq!(exact_match.parameters.len(), 2);
    let matching = [UserEmailFilter {
        address: None,
        insensitive: false,
    }];
    let unfiltered = table.list(RowsFindMany {
        filter: Some(&matching[..]),
        order: None,
        skip: None,
        take: None,
    });
    assert!(!unfiltered.sql.contains("user_email\" ="));
    assert_eq!(unfiltered.parameters.len(), 1);
}

#[test]
fn native_jwt_filter_matches_the_compound_mapping() {
    let table = SelectingTable {
        table: JwtRows,
        include: None,
        limit: Some(1),
    };
    let filters = [JwtMappingFilter {
        claim_name: "role".to_owned(),
        claim_value: "admin".to_owned(),
        issuer: "https://issuer.example".to_owned(),
        active: true,
    }];
    let listed = table.list(RowsFindMany {
        filter: Some(&filters[..]),
        order: None,
        skip: None,
        take: None,
    });
    assert!(listed.sql.contains("entry.\"jwt_claim_name\" = $1"));
    assert!(listed.sql.contains("entry.\"jwt_claim_value\" = $2"));
    assert!(listed.sql.contains("entry.\"jwt_issuer\" = $3"));
    assert!(listed.sql.contains("entry.\"is_active\" = $4"));
    assert!(listed.sql.ends_with("LIMIT $5"));
    assert_eq!(listed.parameters.len(), 5);
}
