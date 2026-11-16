use std::collections::{BTreeMap, BTreeSet};

use betterlite_secrets::secret_codec::diagnostics::SecretDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use secrecy::SecretString;

use crate::budget_permission_rows::JsonData;
use crate::mcp::table_access::lock::ADVISORY_LOCK_SQL;
use crate::mcp::table_access::types::McpTableError;

use super::codec::{decode_user_env_vars, seal_user_env_vars};
use super::error::UserEnvVarsError;
use super::lock::user_env_vars_lock_key;
use super::ports::{
    TransactionFuture, UserEnvVarTable, UserEnvVarsMergeStep, UserEnvVarsTransaction,
    UserEnvVarsTransactions,
};

pub type UserEnvVars = BTreeMap<String, SecretString>;

pub struct UserEnvVarsStore<Table, Transactions, N, D> {
    pub table: Table,
    pub transactions: Transactions,
    pub cipher: StoredCipher<N, D>,
}

impl<Table, Transactions, N, D> UserEnvVarsStore<Table, Transactions, N, D>
where
    Table: UserEnvVarTable,
    Transactions: UserEnvVarsTransactions,
    N: NonceSource + Send + Sync,
    D: SecretDiagnostics + Send + Sync,
{
    pub async fn get_user_env_vars(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<UserEnvVars, UserEnvVarsError> {
        let Some(row) = self.table.find_unique(user_id, server_id).await? else {
            return Ok(UserEnvVars::new());
        };
        Ok(decode_user_env_vars(values_text(&row)?, &self.cipher))
    }

    // an empty id list returns before any query, as LiteLLM skips the pointless round trip
    pub async fn get_user_env_vars_bulk(
        &self,
        user_id: &str,
        server_ids: &[String],
    ) -> Result<BTreeMap<String, UserEnvVars>, UserEnvVarsError> {
        if server_ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        let rows = self.table.find_many(user_id, server_ids).await?;
        let mut found = BTreeMap::new();
        for row in &rows {
            let server_id = text_field(row, "server_id").ok_or(McpTableError::MissingServerId)?;
            let values = decode_user_env_vars(values_text(row)?, &self.cipher);
            found.insert(server_id.to_owned(), values);
        }
        Ok(found)
    }

    pub async fn merge_user_env_vars(
        &self,
        user_id: &str,
        server_id: &str,
        updates: &UserEnvVars,
        allowed_names: &BTreeSet<String>,
    ) -> Result<UserEnvVars, UserEnvVarsError> {
        let step = Merge {
            cipher: &self.cipher,
            user_id,
            server_id,
            updates,
            allowed_names,
        };
        self.transactions.in_transaction(&step).await
    }

    pub async fn delete_user_env_vars(
        &self,
        user_id: &str,
        server_id: &str,
    ) -> Result<(), UserEnvVarsError> {
        let _rows_removed = self.table.delete_many(user_id, server_id).await?;
        Ok(())
    }
}

struct Merge<'a, N, D> {
    cipher: &'a StoredCipher<N, D>,
    user_id: &'a str,
    server_id: &'a str,
    updates: &'a UserEnvVars,
    allowed_names: &'a BTreeSet<String>,
}

impl<N, D> UserEnvVarsMergeStep for Merge<'_, N, D>
where
    N: NonceSource + Send + Sync,
    D: SecretDiagnostics + Send + Sync,
{
    fn apply<'tx>(
        &'tx self,
        transaction: &'tx dyn UserEnvVarsTransaction,
    ) -> TransactionFuture<'tx, UserEnvVars> {
        Box::pin(async move {
            // the advisory lock is taken before the read so two merges cannot lose one update
            let key = user_env_vars_lock_key(self.user_id, self.server_id);
            transaction.execute_raw(ADVISORY_LOCK_SQL, key).await?;
            let row = transaction
                .find_unique(self.user_id, self.server_id)
                .await?;
            let mut merged = UserEnvVars::new();
            if let Some(row) = row {
                merged = decode_user_env_vars(values_text(&row)?, self.cipher);
            }
            for (name, value) in self.updates {
                merged.insert(name.clone(), value.clone());
            }
            merged.retain(|name, _| self.allowed_names.contains(name));
            let sealed = seal_user_env_vars(&merged, self.cipher)?;
            transaction
                .upsert(self.user_id, self.server_id, &sealed)
                .await?;
            Ok(merged)
        })
    }
}

// values_b64 is a non-null text column, so a row without one is an error and not an unset value
pub(crate) fn values_text(row: &JsonData) -> Result<&str, UserEnvVarsError> {
    match text_field(row, "values_b64") {
        Some(text) => Ok(text),
        None => Err(UserEnvVarsError::MissingValues),
    }
}

fn text_field<'row>(row: &'row JsonData, name: &str) -> Option<&'row str> {
    let JsonData::Object(fields) = row else {
        return None;
    };
    match fields.get(name)? {
        JsonData::String(text) => Some(text),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => None,
    }
}
