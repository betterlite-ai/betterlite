use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

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
    fn record(&mut self, event: &str) {
        self.events.push(event.to_owned());
    }

    fn find(&self, user_id: &str, server_id: &str) -> Option<JsonData> {
        self.rows
            .iter()
            .find(|row| {
                text(row, "user_id") == Some(user_id) && text(row, "server_id") == Some(server_id)
            })
            .cloned()
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
        self.rows.push(row_json(user_id, server_id, values_b64));
    }
}

fn row_json(user_id: &str, server_id: &str, values_b64: &str) -> JsonData {
    JsonData::Object(BTreeMap::from([
        ("user_id".to_owned(), JsonData::String(user_id.to_owned())),
        (
            "server_id".to_owned(),
            JsonData::String(server_id.to_owned()),
        ),
        (
            "values_b64".to_owned(),
            JsonData::String(values_b64.to_owned()),
        ),
    ]))
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
    async fn find_unique(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<Option<JsonData>, UserEnvVarsError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.record("find_unique");
        Ok(state.find(user_id, server_id))
    }

    async fn find_many(
        &self,
        user_id: &str,
        server_ids: &[String],
    ) -> Result<Vec<JsonData>, UserEnvVarsError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.record("find_many");
        Ok(state
            .rows
            .iter()
            .filter(|row| {
                text(row, "user_id") == Some(user_id)
                    && server_ids
                        .iter()
                        .any(|wanted| text(row, "server_id") == Some(wanted.as_str()))
            })
            .cloned()
            .collect())
    }

    async fn delete_many(&self, user_id: &str, server_id: &str) -> Result<usize, UserEnvVarsError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.record("delete_many");
        let before = state.rows.len();
        state.rows.retain(|row| {
            text(row, "user_id") != Some(user_id) || text(row, "server_id") != Some(server_id)
        });
        Ok(before.saturating_sub(state.rows.len()))
    }
}

struct RowsTransaction {
    state: Arc<Mutex<RowState>>,
}

impl UserEnvVarsTransaction for RowsTransaction {
    fn execute_raw<'a>(&'a self, _statement: &'a str, key: i64) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.record("lock");
            state.locks.push(key);
            Ok(())
        })
    }

    fn find_unique<'a>(
        &'a self,
        user_id: &'a str,
        server_id: &'a str,
    ) -> TransactionFuture<'a, Option<JsonData>> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.record("find_unique");
            Ok(state.find(user_id, server_id))
        })
    }

    fn upsert<'a>(
        &'a self,
        user_id: &'a str,
        server_id: &'a str,
        values_b64: &'a str,
    ) -> TransactionFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.record("upsert");
            state.put_row(user_id, server_id, values_b64);
            Ok(())
        })
    }
}

impl UserEnvVarsTransactions for RowStore {
    fn in_transaction<'a>(
        &'a self,
        step: &'a dyn UserEnvVarsMergeStep,
    ) -> TransactionFuture<'a, UserEnvVars> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record("tx");
        let transaction = RowsTransaction {
            state: Arc::clone(&self.state),
        };
        Box::pin(async move { step.apply(&transaction).await })
    }
}
