use std::sync::Arc;

use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};

use super::backend::NativeHealthBackend;
use super::documents::OcrDocuments;
use crate::health_check_entry::HealthCheckEntry;
use crate::health_check_entry::ports::{CatalogModes, ProviderResolver};
use crate::health_probe_modes::backend::{CheapestChatModels, HealthProbeBackend};

// the values the server already holds, handed in so the entry builds no global and reads no environment
pub struct HealthEntryDeps {
    pub backend: NativeHealthBackend,
    pub resolver: Arc<dyn ProviderResolver>,
    pub catalog: Arc<dyn CatalogModes>,
    pub redactor: Arc<Redactor>,
    pub redact_secrets: bool,
    pub clock: fn() -> DateTime<FixedOffset>,
    pub next_id: fn() -> String,
}

#[must_use]
pub fn build_health_check_entry(deps: HealthEntryDeps) -> HealthCheckEntry {
    let backend = Arc::new(deps.backend);
    let health: Arc<dyn HealthProbeBackend> = Arc::<NativeHealthBackend>::clone(&backend);
    let cheapest: Arc<dyn CheapestChatModels> = backend;
    HealthCheckEntry {
        backend: health,
        cheapest,
        documents: Arc::new(OcrDocuments),
        resolver: deps.resolver,
        catalog: deps.catalog,
        redactor: deps.redactor,
        redact_secrets: deps.redact_secrets,
        clock: deps.clock,
        next_id: deps.next_id,
    }
}
