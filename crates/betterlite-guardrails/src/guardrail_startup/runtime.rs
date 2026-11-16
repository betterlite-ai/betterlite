use betterlite_config::sections::guardrail_params::records::Guardrail;
use tokio::sync::{Mutex, RwLock};

use super::reconcile::GuardrailReconcile as _;
use crate::guardrail_registry::error::RegistryError;
use crate::guardrail_registry::handler::InMemoryGuardrailHandler;
use crate::guardrail_registry::registry::GuardrailRegistry;
use crate::guardrail_registry::rows::GuardrailRows;

pub struct GuardrailRuntime<R: GuardrailRows> {
    pub(super) handler: RwLock<InMemoryGuardrailHandler>,
    pub(super) registry: GuardrailRegistry<R>,
    // LiteLLM's GUARDRAIL_RECONCILE_LOCK: it serialises the writers of the poll and read-through paths
    pub(super) reconcile_lock: Mutex<()>,
}

impl<R: GuardrailRows> GuardrailRuntime<R> {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn new(handler: InMemoryGuardrailHandler, registry: GuardrailRegistry<R>) -> Self {
        Self {
            handler: RwLock::new(handler),
            registry,
            reconcile_lock: Mutex::new(()),
        }
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub const fn handler(&self) -> &RwLock<InMemoryGuardrailHandler> {
        &self.handler
    }

    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub const fn registry(&self) -> &GuardrailRegistry<R> {
        &self.registry
    }

    // Mutex first, write lock second, and never hold the write lock across a database await
    pub async fn sync_locked(
        &self,
        guardrail: Guardrail,
    ) -> Result<Option<Guardrail>, RegistryError> {
        let _serialised = self.reconcile_lock.lock().await;
        self.handler.write().await.sync_guardrail_from_db(guardrail)
    }
}
