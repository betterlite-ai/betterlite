use std::thread::Builder;

use tokio::runtime::Handle;

use super::purview_async_logging_hook::{AuditEngine, DeferredCall};

pub trait AuditSpawner {
    fn spawn(&self, job: Box<dyn FnOnce() + Send + 'static>);
}

pub struct ThreadAuditSpawner;

impl AuditSpawner for ThreadAuditSpawner {
    fn spawn(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        let spawned = Builder::new()
            .name("purview-dlp-audit".to_owned())
            .spawn(job);
        if let Err(error) = spawned {
            tracing::error!("Purview audit background logging error: {error}");
        }
    }
}

// The sync hook returns at once: an async caller's framework runs async_logging_hook, and otherwise a background thread runs the audit.
#[must_use]
pub fn logging_hook(
    engine: &AuditEngine,
    call: DeferredCall,
    spawner: &dyn AuditSpawner,
) -> DeferredCall {
    if Handle::try_current().is_ok() {
        tracing::debug!(
            "Purview audit: deferring to async_logging_hook (running event loop detected)"
        );
        return call;
    }
    let job_engine = engine.clone();
    let job_call = call.clone();
    spawner.spawn(Box::new(move || job_engine.audit_blocking(job_call)));
    call
}

mod tests;
