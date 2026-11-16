use std::future::Future;

use betterlite_config::sections::guardrail_params::records::Guardrail;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::callback::GuardrailCallback;
use super::error::{RegistryError, wrapped};
use super::handler::InMemoryGuardrailHandler;
use super::rows::{
    GuardrailRow, GuardrailRows, RowsError, guardrail_to_new_row, guardrail_to_update,
    row_to_guardrail,
};

const ADD_CONTEXT: &str = "Error adding guardrail to DB";
const DELETE_CONTEXT: &str = "Error deleting guardrail from DB";
const UPDATE_CONTEXT: &str = "Error updating guardrail in DB";
const GET_ALL_CONTEXT: &str = "Error getting guardrails from DB";
const GET_ONE_CONTEXT: &str = "Error getting guardrail from DB";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteMessage {
    pub message: String,
}

pub struct GuardrailRegistry<R: GuardrailRows> {
    rows: R,
}

impl<R: GuardrailRows> GuardrailRegistry<R> {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub const fn new(rows: R) -> Self {
        Self { rows }
    }

    pub async fn add_guardrail_to_db(
        &self,
        guardrail: &Guardrail,
        now: DateTime<Utc>,
    ) -> Result<Guardrail, RegistryError> {
        let created = self
            .rows
            .create(guardrail_to_new_row(guardrail, now)?)
            .await
            .map_err(|source| wrapped(ADD_CONTEXT, source))?;
        Ok(Guardrail {
            guardrail_id: Some(created.guardrail_id),
            ..guardrail.clone()
        })
    }

    pub async fn delete_guardrail_from_db(
        &self,
        guardrail_id: &str,
    ) -> Result<DeleteMessage, RegistryError> {
        let deleted = self.rows.delete(guardrail_id).await;
        deleted.map_err(|source| wrapped(DELETE_CONTEXT, source))?;
        Ok(DeleteMessage {
            message: format!("Guardrail {guardrail_id} deleted successfully"),
        })
    }

    pub async fn update_guardrail_in_db(
        &self,
        guardrail_id: &str,
        guardrail: &Guardrail,
        now: DateTime<Utc>,
    ) -> Result<Guardrail, RegistryError> {
        let update = guardrail_to_update(guardrail, now)?;
        let updated = self
            .rows
            .update(guardrail_id, update)
            .await
            .map_err(|source| wrapped(UPDATE_CONTEXT, source))?;
        let Some(updated_row) = updated else {
            let missing = RegistryError::UpdateNotFound {
                guardrail_id: guardrail_id.to_owned(),
            };
            return Err(wrapped(UPDATE_CONTEXT, missing));
        };
        row_to_guardrail(&updated_row).map_err(|source| wrapped(UPDATE_CONTEXT, source))
    }

    pub async fn get_all_guardrails_from_db(&self) -> Result<Vec<Guardrail>, RegistryError> {
        let rows = self
            .rows
            .find_active()
            .await
            .map_err(|source| wrapped(GET_ALL_CONTEXT, source))?;
        let mut guardrails = Vec::with_capacity(rows.len());
        for row in &rows {
            let guardrail =
                row_to_guardrail(row).map_err(|source| wrapped(GET_ALL_CONTEXT, source))?;
            guardrails.push(guardrail);
        }
        Ok(guardrails)
    }

    // ast-grep-ignore: no-one-line-functions
    pub async fn get_guardrail_by_id_from_db(
        &self,
        guardrail_id: &str,
    ) -> Result<Option<Guardrail>, RegistryError> {
        stored_guardrail(self.rows.find_by_id(guardrail_id)).await
    }

    // ast-grep-ignore: no-one-line-functions
    pub async fn get_guardrail_by_name_from_db(
        &self,
        guardrail_name: &str,
    ) -> Result<Option<Guardrail>, RegistryError> {
        stored_guardrail(self.rows.find_by_name(guardrail_name)).await
    }

    // ast-grep-ignore: no-long-names
    pub fn get_initialized_guardrail_callback<'handler>(
        &self,
        handler: &'handler InMemoryGuardrailHandler,
        guardrail_name: &str,
    ) -> Option<&'handler GuardrailCallback> {
        handler.all_callbacks().find(|callback| {
            callback.base.settings.guardrail_name.as_deref() == Some(guardrail_name)
        })
    }
}

async fn stored_guardrail<F>(found: F) -> Result<Option<Guardrail>, RegistryError>
where
    F: Future<Output = Result<Option<GuardrailRow>, RowsError>> + Send,
{
    let row = found
        .await
        .map_err(|source| wrapped(GET_ONE_CONTEXT, source))?;
    row.as_ref()
        .map(row_to_guardrail)
        .transpose()
        .map_err(|source| wrapped(GET_ONE_CONTEXT, source))
}
