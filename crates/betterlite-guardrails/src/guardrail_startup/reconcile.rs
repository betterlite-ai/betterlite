use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_params::records::{
    Guardrail, GuardrailDefinitionLocation,
};

use crate::guardrail_registry::changed::params_changed;
use crate::guardrail_registry::error::RegistryError;
use crate::guardrail_registry::handler::InMemoryGuardrailHandler;

// the workspace denies multiple_inherent_impl, so the handler's two sync methods live on this trait
pub trait GuardrailReconcile {
    fn reconcile_db_guardrails(&mut self, db_guardrail_ids: &BTreeSet<String>) -> Vec<String>;

    fn sync_guardrail_from_db(
        &mut self,
        guardrail: Guardrail,
    ) -> Result<Option<Guardrail>, RegistryError>;
}

impl GuardrailReconcile for InMemoryGuardrailHandler {
    fn reconcile_db_guardrails(&mut self, db_guardrail_ids: &BTreeSet<String>) -> Vec<String> {
        let stale_ids: Vec<String> = self
            .list_in_memory_guardrails()
            .into_iter()
            .filter_map(|guardrail| guardrail.guardrail_id)
            .filter(|guardrail_id| {
                self.get_source(guardrail_id) == Some(GuardrailDefinitionLocation::Db)
                    && !db_guardrail_ids.contains(guardrail_id)
            })
            .collect();
        for guardrail_id in &stale_ids {
            tracing::info!(
                "Reconcile: removing stale DB-backed guardrail '{guardrail_id}' from memory (deleted in DB by another pod)"
            );
            self.delete_in_memory_guardrail(guardrail_id);
        }
        stale_ids
    }

    fn sync_guardrail_from_db(
        &mut self,
        guardrail: Guardrail,
    ) -> Result<Option<Guardrail>, RegistryError> {
        let Some(guardrail_id) = guardrail.guardrail_id.clone().filter(|id| !id.is_empty()) else {
            tracing::error!("Cannot sync guardrail without guardrail_id");
            return Ok(None);
        };
        if params_changed(self.get_guardrail_by_id(&guardrail_id), &guardrail)? {
            tracing::info!(
                "Guardrail '{}' (ID: {guardrail_id}) params changed, re-initializing...",
                guardrail.guardrail_name
            );
            return self.reinitialize_guardrail(&guardrail, GuardrailDefinitionLocation::Db);
        }
        // the id is held already, so this builds nothing and only moves the source marker to db
        let held = self.initialize_guardrail(guardrail, GuardrailDefinitionLocation::Db)?;
        Ok(Some(held))
    }
}
