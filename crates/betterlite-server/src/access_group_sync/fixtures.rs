#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_db::postgres_store::errors::DbError;
use rust_decimal::Decimal;
use tokio::sync::Barrier;

use super::cache::AccessGroupCache;
use super::key::{KeyMembershipDatabase, KeyMembershipStatement, KeySyncError, MovedGroupRow};

pub(crate) mod team;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyCall {
    Move {
        statement: KeyMembershipStatement,
        key_token: String,
        access_group_ids: Vec<String>,
    },
    Repoint {
        previous_key_token: String,
        new_key_token: String,
    },
}

// Stands in for the two guarded statements, including their NULL-array and membership guards
pub(crate) struct ScriptedKeyDatabase {
    pub(crate) groups: Mutex<BTreeMap<String, Vec<String>>>,
    pub(crate) calls: Mutex<Vec<KeyCall>>,
    pub(crate) reverse_rows: bool,
    pub(crate) fail_statement: Option<KeyMembershipStatement>,
}

impl ScriptedKeyDatabase {
    pub(crate) const fn new(groups: BTreeMap<String, Vec<String>>) -> Self {
        Self {
            groups: Mutex::new(groups),
            calls: Mutex::new(Vec::new()),
            reverse_rows: false,
            fail_statement: None,
        }
    }

    fn move_rows(
        &self,
        statement: KeyMembershipStatement,
        key_token: &str,
        access_group_ids: &[String],
    ) -> Vec<MovedGroupRow> {
        let mut groups = self.groups.lock().expect("groups");
        let mut moved = Vec::new();
        for access_group_id in access_group_ids {
            let Some(tokens) = groups.get_mut(access_group_id) else {
                continue;
            };
            let present = tokens.iter().any(|token| token == key_token);
            match statement {
                KeyMembershipStatement::Attach if !present => {
                    tokens.push(key_token.to_owned());
                    moved.push(MovedGroupRow {
                        access_group_id: access_group_id.clone(),
                    });
                }
                KeyMembershipStatement::Detach if present => {
                    tokens.retain(|token| token != key_token);
                    moved.push(MovedGroupRow {
                        access_group_id: access_group_id.clone(),
                    });
                }
                KeyMembershipStatement::Attach | KeyMembershipStatement::Detach => {}
            }
        }
        if self.reverse_rows {
            moved.reverse();
        }
        moved
    }

    fn repoint_rows(&self, previous: &str, new: &str) -> Vec<MovedGroupRow> {
        let mut groups = self.groups.lock().expect("groups");
        let mut moved = Vec::new();
        for (access_group_id, tokens) in groups.iter_mut() {
            if !tokens.iter().any(|token| token == previous) {
                continue;
            }
            tokens.retain(|token| token != previous && token != new);
            tokens.push(new.to_owned());
            moved.push(MovedGroupRow {
                access_group_id: access_group_id.clone(),
            });
        }
        if self.reverse_rows {
            moved.reverse();
        }
        moved
    }
}

impl KeyMembershipDatabase for ScriptedKeyDatabase {
    fn move_key_membership(
        &self,
        statement: KeyMembershipStatement,
        key_token: &str,
        access_group_ids: &[String],
    ) -> impl Future<Output = Result<Vec<MovedGroupRow>, KeySyncError>> + Send {
        self.calls.lock().expect("calls").push(KeyCall::Move {
            statement,
            key_token: key_token.to_owned(),
            access_group_ids: access_group_ids.to_vec(),
        });
        let failed = self.fail_statement == Some(statement);
        let moved = if failed {
            Vec::new()
        } else {
            self.move_rows(statement, key_token, access_group_ids)
        };
        async move {
            if failed {
                return Err(KeySyncError::Database(DbError::NoDbConnected));
            }
            Ok(moved)
        }
    }

    fn repoint_key_membership(
        &self,
        previous: &str,
        new: &str,
    ) -> impl Future<Output = Result<Vec<MovedGroupRow>, KeySyncError>> + Send {
        self.calls.lock().expect("calls").push(KeyCall::Repoint {
            previous_key_token: previous.to_owned(),
            new_key_token: new.to_owned(),
        });
        let moved = self.repoint_rows(previous, new);
        async move { Ok(moved) }
    }
}

// Records eviction order, and fails the listed ids with distinct errors when the test asks
pub(crate) struct ScriptedCache {
    pub(crate) calls: Mutex<Vec<String>>,
    pub(crate) failures: Mutex<BTreeMap<String, i64>>,
    pub(crate) barrier: Option<Arc<Barrier>>,
}

impl ScriptedCache {
    pub(crate) const fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            failures: Mutex::new(BTreeMap::new()),
            barrier: None,
        }
    }
}

impl AccessGroupCache for ScriptedCache {
    fn delete_access_group(
        &self,
        access_group_id: &str,
    ) -> impl Future<Output = Result<(), InvalidationError>> + Send {
        self.calls
            .lock()
            .expect("calls")
            .push(access_group_id.to_owned());
        let failure = self
            .failures
            .lock()
            .expect("failures")
            .get(access_group_id)
            .copied();
        let barrier = self.barrier.clone();
        async move {
            if let Some(gate) = barrier {
                let _arrived = gate.wait().await;
            }
            if let Some(code) = failure {
                return Err(InvalidationError::SpendOutOfRange(Decimal::new(code, 0)));
            }
            Ok(())
        }
    }
}
