use std::sync::{Mutex, PoisonError};

// LiteLLM keeps the first project id Vertex resolves, and a configured id always wins over it
#[derive(Default)]
pub struct ResolvedProject(Mutex<Option<String>>);

impl ResolvedProject {
    #[must_use]
    pub fn effective(&self, configured: Option<&str>) -> Option<String> {
        if let Some(configured_id) = configured.filter(|id| !id.is_empty()) {
            return Some(configured_id.to_owned());
        }
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn remember(&self, configured: Option<&str>, resolved: Option<&str>) {
        if configured.is_some_and(|id| !id.is_empty()) {
            return;
        }
        let Some(resolved_id) = resolved.filter(|id| !id.is_empty()) else {
            return;
        };
        let mut remembered = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if remembered.is_none() {
            *remembered = Some(resolved_id.to_owned());
        }
    }
}
