use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use tokio_util::task::TaskTracker;

use super::payload::AuditLogPayload;

pub type AuditCallbackFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuditCallbackError {
    #[error("the audit log callback failed")]
    Failed,
}

#[async_trait]
pub trait AuditLogCallback: Send + Sync {
    async fn async_log_audit_log_event(
        &self,
        payload: AuditLogPayload,
    ) -> Result<(), AuditCallbackError>;
}

#[derive(Clone)]
pub enum AuditCallbackConfig {
    Instance(Arc<dyn AuditLogCallback>),
    Named(String),
    Other,
}

pub trait AuditCallbackRegistry: Send + Sync {
    fn resolve(&self, name: &str) -> Option<Arc<dyn AuditLogCallback>>;

    fn build_dedicated_s3_logger(
        &self,
        params: &JsonValue,
    ) -> Result<Arc<dyn AuditLogCallback>, AuditCallbackError>;
}

pub trait AuditTaskSpawner: Send + Sync {
    fn spawn(&self, task: AuditCallbackFuture);
}

pub struct TrackerSpawner {
    pub tasks: TaskTracker,
}

impl AuditTaskSpawner for TrackerSpawner {
    fn spawn(&self, task: AuditCallbackFuture) {
        self.tasks.spawn(task);
    }
}

pub struct AuditCallbackDispatcher {
    pub registry: Arc<dyn AuditCallbackRegistry>,
    pub spawner: Arc<dyn AuditTaskSpawner>,
    cache: Mutex<BTreeMap<String, Arc<dyn AuditLogCallback>>>,
}

impl AuditCallbackDispatcher {
    #[must_use]
    pub fn new(
        registry: Arc<dyn AuditCallbackRegistry>,
        spawner: Arc<dyn AuditTaskSpawner>,
    ) -> Self {
        Self {
            registry,
            spawner,
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn reset_audit_log_callback_cache(&self) {
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    pub fn dispatch(
        &self,
        callbacks: &[AuditCallbackConfig],
        s3_params: Option<&JsonValue>,
        payload: AuditLogPayload,
    ) {
        for config in callbacks {
            let Some(callback) = self.resolve_config(config, s3_params) else {
                continue;
            };
            let payload = payload.clone();
            let task: AuditCallbackFuture = Box::pin(async move {
                if let Err(error) = callback.async_log_audit_log_event(payload).await {
                    tracing::error!("Audit log callback task failed: {error}");
                }
            });
            self.spawner.spawn(task);
        }
    }

    fn resolve_config(
        &self,
        config: &AuditCallbackConfig,
        s3_params: Option<&JsonValue>,
    ) -> Option<Arc<dyn AuditLogCallback>> {
        match config {
            AuditCallbackConfig::Instance(callback) => Some(Arc::clone(callback)),
            AuditCallbackConfig::Named(name) => self.resolve_name(name, s3_params),
            // LiteLLM silently ignores a configured value that is neither a logger nor a name
            AuditCallbackConfig::Other => None,
        }
    }

    fn resolve_name(
        &self,
        name: &str,
        s3_params: Option<&JsonValue>,
    ) -> Option<Arc<dyn AuditLogCallback>> {
        if let Some(cached) = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
        {
            return Some(cached);
        }
        let resolved = match (name, s3_params) {
            ("s3_v2", Some(params)) => match self.registry.build_dedicated_s3_logger(params) {
                Ok(callback) => callback,
                Err(error) => {
                    tracing::error!("Failed dispatching audit log to callback: {error}");
                    return None;
                }
            },
            _ => match self.registry.resolve(name) {
                Some(callback) => callback,
                None => {
                    tracing::warn!("Could not resolve audit log callback: {name}");
                    return None;
                }
            },
        };
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), Arc::clone(&resolved));
        Some(resolved)
    }
}
