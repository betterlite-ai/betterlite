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
use super::team::{AffectedGroupRow, TeamGroupRow, TeamSyncError, TeamSyncTransaction};

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
        previous_key_token: &str,
        new_key_token: &str,
    ) -> impl Future<Output = Result<Vec<MovedGroupRow>, KeySyncError>> + Send {
        self.calls.lock().expect("calls").push(KeyCall::Repoint {
            previous_key_token: previous_key_token.to_owned(),
            new_key_token: new_key_token.to_owned(),
        });
        let moved = self.repoint_rows(previous_key_token, new_key_token);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeamStep {
    Lock,
    Read,
    Affected,
    Attach,
    Detach,
}

pub(crate) struct ScriptedTeamTransaction {
    pub(crate) calls: Mutex<Vec<TeamStep>>,
    pub(crate) missing_team: bool,
    pub(crate) team_group_ids: Option<Vec<String>>,
    pub(crate) groups: Mutex<BTreeMap<String, Option<Vec<String>>>>,
}

impl ScriptedTeamTransaction {
    pub(crate) const fn new(
        team_group_ids: Option<Vec<String>>,
        groups: BTreeMap<String, Option<Vec<String>>>,
    ) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            missing_team: false,
            team_group_ids,
            groups: Mutex::new(groups),
        }
    }
}

impl TeamSyncTransaction for ScriptedTeamTransaction {
    fn lock_team(&self, _team_id: &str) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        self.calls.lock().expect("calls").push(TeamStep::Lock);
        async move { Ok(()) }
    }

    fn team_groups(
        &self,
        _team_id: &str,
    ) -> impl Future<Output = Result<Vec<TeamGroupRow>, TeamSyncError>> + Send {
        self.calls.lock().expect("calls").push(TeamStep::Read);
        let rows = if self.missing_team {
            Vec::new()
        } else {
            vec![TeamGroupRow {
                access_group_ids: self.team_group_ids.clone(),
            }]
        };
        async move { Ok(rows) }
    }

    fn affected_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<Vec<AffectedGroupRow>, TeamSyncError>> + Send {
        self.calls.lock().expect("calls").push(TeamStep::Affected);
        let groups = self.groups.lock().expect("groups");
        let rows = groups
            .iter()
            .filter(|(access_group_id, assigned)| {
                desired.contains(access_group_id)
                    || assigned
                        .as_ref()
                        .is_some_and(|teams| teams.iter().any(|team| team == team_id))
            })
            .map(|(access_group_id, _)| AffectedGroupRow {
                access_group_id: access_group_id.clone(),
            })
            .collect();
        async move { Ok(rows) }
    }

    fn attach_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        self.calls.lock().expect("calls").push(TeamStep::Attach);
        let mut groups = self.groups.lock().expect("groups");
        for access_group_id in desired {
            let Some(assigned) = groups.get_mut(access_group_id) else {
                continue;
            };
            let teams = assigned.get_or_insert_default();
            if !teams.iter().any(|team| team == team_id) {
                teams.push(team_id.to_owned());
            }
        }
        async move { Ok(()) }
    }

    fn detach_groups(
        &self,
        team_id: &str,
        desired: &[String],
    ) -> impl Future<Output = Result<(), TeamSyncError>> + Send {
        self.calls.lock().expect("calls").push(TeamStep::Detach);
        let mut groups = self.groups.lock().expect("groups");
        for (access_group_id, assigned) in groups.iter_mut() {
            if desired.contains(access_group_id) {
                continue;
            }
            let Some(teams) = assigned else {
                continue;
            };
            teams.retain(|team| team != team_id);
        }
        async move { Ok(()) }
    }
}
