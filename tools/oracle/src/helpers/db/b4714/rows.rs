use std::collections::BTreeMap;
use std::future::Future;
use std::future::ready;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp_user_env_vars::error::UserEnvVarsError;
use betterlite_db::mcp_user_env_vars::ports::{
    TransactionFuture, UserEnvVarTable, UserEnvVarsMergeStep, UserEnvVarsTransaction,
    UserEnvVarsTransactions,
};
use betterlite_db::mcp_user_env_vars::store::UserEnvVars;

// an in-memory stand-in for the Prisma env var table: it answers the composite key and the in-list
#[derive(Clone)]
pub(super) struct RowStore {
    state: Arc<Mutex<RowState>>,
}

struct RowState {
    rows: Vec<JsonData>,
    events: Vec<String>,
    locks: Vec<i64>,
}

fn recording<'state>(state: &'state Mutex<RowState>, event: &str) -> MutexGuard<'state, RowState> {
    let mut guard = state.lock().unwrap_or_else(PoisonError::into_inner);
    guard.events.push(event.to_owned());
    guard
}

impl RowStore {
    pub(super) fn new(rows: &[JsonData]) -> Self {
        Self {
            state: Arc::new(Mutex::new(RowState {
                rows: rows.to_vec(),
                events: Vec::new(),
                locks: Vec::new(),
            })),
        }
    }

    pub(super) fn rows(&self) -> Vec<JsonData> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rows
            .clone()
    }

    pub(super) fn events(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .events
            .clone()
    }

    pub(super) fn locks(&self) -> Vec<i64> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .locks
            .clone()
    }
}

impl RowState {
    fn find(&self, user_id: &str, server_id: &str) -> Option<JsonData> {
        self.rows
            .iter()
            .find(|row| {
                text(row, "user_id") == Some(user_id) && text(row, "server_id") == Some(server_id)
            })
            .cloned()
    }

    fn select(&self, user_id: &str, server_ids: &[String]) -> Vec<JsonData> {
        let mut selected = Vec::new();
        for row in &self.rows {
            let Some(row_user) = text(row, "user_id") else {
                continue;
            };
            let Some(row_server) = text(row, "server_id") else {
                continue;
            };
            if row_user == user_id && server_ids.iter().any(|wanted| wanted == row_server) {
                selected.push(row.clone());
            }
        }
        selected
    }

    fn put_row(&mut self, user_id: &str, server_id: &str, values_b64: &str) {
        for row in &mut self.rows {
            if text(row, "user_id") == Some(user_id) && text(row, "server_id") == Some(server_id) {
                if let JsonData::Object(fields) = row {
                    fields.insert(
                        "values_b64".to_owned(),
                        JsonData::String(values_b64.to_owned()),
                    );
                }
                return;
            }
        }
        let mut fields = BTreeMap::new();
        fields.insert("user_id".to_owned(), JsonData::String(user_id.to_owned()));
        fields.insert(
            "server_id".to_owned(),
            JsonData::String(server_id.to_owned()),
        );
        fields.insert(
            "values_b64".to_owned(),
            JsonData::String(values_b64.to_owned()),
        );
        self.rows.push(JsonData::Object(fields));
    }
}

pub(super) fn text<'row>(row: &'row JsonData, name: &str) -> Option<&'row str> {
    let JsonData::Object(fields) = row else {
        return None;
    };
    match fields.get(name)? {
        JsonData::String(value) => Some(value),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}

impl UserEnvVarTable for RowStore {
    fn find_unique(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<Option<JsonData>, UserEnvVarsError>> + Send {
        let state = recording(&self.state, "find_unique");
        ready(Ok(state.find(user_id, server_id)))
    }

    fn find_many(
        &self,
        user_id: &str,
        server_ids: &[String],
    ) -> impl Future<Output = Result<Vec<JsonData>, UserEnvVarsError>> + Send {
        let state = recording(&self.state, "find_many");
        ready(Ok(state.select(user_id, server_ids)))
    }

    fn delete_many(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> impl Future<Output = Result<(), UserEnvVarsError>> + Send {
        let mut state = recording(&self.state, "delete_many");
        state.rows.retain(|row| {
            text(row, "user_id") != Some(user_id) || text(row, "server_id") != Some(server_id)
        });
        drop(state);
        ready(Ok(()))
    }
}

struct RowsTransaction {
    store: RowStore,
}

impl UserEnvVarsTransaction for RowsTransaction {
    fn execute_raw<'tx>(&'tx self, _statement: &'tx str, key: i64) -> TransactionFuture<'tx, ()> {
        Box::pin(async move {
            let mut state = recording(&self.store.state, "lock");
            state.locks.push(key);
            drop(state);
            Ok(())
        })
    }

    fn find_unique<'tx>(
        &'tx self,
        user_id: &'tx str,
        server_id: &'tx str,
    ) -> TransactionFuture<'tx, Option<JsonData>> {
        Box::pin(async move { self.store.find_unique(user_id, server_id).await })
    }

    fn upsert<'tx>(
        &'tx self,
        user_id: &'tx str,
        server_id: &'tx str,
        values_b64: &'tx str,
    ) -> TransactionFuture<'tx, ()> {
        Box::pin(async move {
            let mut state = recording(&self.store.state, "upsert");
            state.put_row(user_id, server_id, values_b64);
            drop(state);
            Ok(())
        })
    }
}

impl UserEnvVarsTransactions for RowStore {
    fn in_transaction<'tx>(
        &'tx self,
        step: &'tx dyn UserEnvVarsMergeStep,
    ) -> TransactionFuture<'tx, UserEnvVars> {
        drop(recording(&self.state, "tx"));
        let transaction = RowsTransaction {
            store: self.clone(),
        };
        Box::pin(async move { step.apply(&transaction).await })
    }
}
