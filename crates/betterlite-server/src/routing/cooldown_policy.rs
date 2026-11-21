use std::sync::Arc;

use betterlite_router::cooldown_policy::event::CooldownObserver;

mod tests;

// the async success and global callback lists, read the way _get_prometheus_logger_from_callbacks reads them
pub struct CooldownObservers {
    pub success: Vec<Arc<dyn CooldownObserver>>,
    pub global: Vec<Arc<dyn CooldownObserver>>,
}

impl CooldownObservers {
    #[must_use]
    pub fn new(
        success: Vec<Arc<dyn CooldownObserver>>,
        global: Vec<Arc<dyn CooldownObserver>>,
    ) -> Self {
        Self { success, global }
    }

    // _get_prometheus_logger_from_callbacks: the async success list is searched first, then the global one
    #[must_use]
    pub fn exporter(&self) -> Option<Arc<dyn CooldownObserver>> {
        if let Some(observer) = self.success.first() {
            return Some(Arc::clone(observer));
        }
        self.global.first().map(Arc::clone)
    }

    // a cooldown reaches the one exporter, since LiteLLM calls a single Prometheus logger per event
    #[must_use]
    pub fn handlers(&self) -> Vec<Arc<dyn CooldownObserver>> {
        let exporter = self.exporter();
        exporter.into_iter().collect()
    }
}
