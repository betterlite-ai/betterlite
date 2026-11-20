pub mod agentops;
pub mod argilla;
pub mod arize;
pub mod arize_phoenix;
pub mod athina;
pub mod aws_sqs;
pub mod azure_sentinel;
pub mod azure_storage;
pub mod bitbucket;
pub mod braintrust;
pub mod clickhouse;
pub mod cloudzero;
pub mod datadog_cost_management;
pub mod datadog_llm_observability;
pub mod datadog_metrics;
pub mod deepeval;
pub mod dotprompt;
pub mod dynamodb;
pub mod focus;
pub mod galileo;
pub mod gcs_bucket;
pub mod gcs_pubsub;
pub mod gitlab;
pub mod greenscale;
pub mod helicone;
pub mod humanloop;
pub mod lago;
pub mod langfuse;
pub mod langfuse_otel;
pub mod langsmith;
pub mod langtrace;
pub mod levo;
pub mod literalai;
pub mod logfire;
pub mod lunary;
pub mod mavvrik;
pub mod mlflow;
pub mod newrelic;
pub mod openmeter;
pub mod opik;
pub mod otel;
pub mod pagerduty;
pub mod pointfive;
pub mod posthog;
pub mod promptlayer;
pub mod resend_email;
pub mod s3;
pub mod s3_v2;
pub mod sendgrid_email;
pub mod sentry;
pub mod signoz;
pub mod slack;
pub mod smtp_email;
pub mod supabase;
mod tests;
pub mod traceloop;
pub mod vantage;
pub mod wandb;
pub mod weave_otel;
pub mod zerobus;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::callback_settings::CallbackEntry;
use betterlite_config::sections::callback_settings::names::CallbackName;
use betterlite_config::types::Config;
use betterlite_http::proxy::ProxiedClient;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use self::otel::phoenix_auto;
use self::otel::shared::SharedOtel;
use super::error::RegistryError;
use crate::callback_config::error::ValidationError;
use crate::callback_config::family::{conflicting_span_scope_error, cross_entry_family_error};
use crate::callback_config::options::callback_config_error;
use crate::callback_config::{CallbackVars, FeatureFlags};
use crate::callback_logger::CallbackLogger;
use crate::callback_logger::resolver::LoggerResolver;
use crate::otel::logger_runtime::LoggerRuntime;

pub fn validate_callback_vars(
    callback_name: Option<&str>,
    vars: &CallbackVars,
    stored: &[CallbackVars],
    flags: FeatureFlags,
) -> Result<(), ValidationError> {
    callback_config_error(callback_name, vars, flags)?;
    cross_entry_family_error(vars, stored)?;
    conflicting_span_scope_error(vars, stored)
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("no callback adapter is installed for '{name}'")]
    Unavailable { name: String },
    #[error("{message}")]
    Init { message: String },
}

pub struct CatalogInputs {
    pub config: Arc<Config>,
    pub http: ProxiedClient,
    pub env: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub callback_settings: BTreeMap<String, CallbackEntry>,
    pub tracker: TaskTracker,
    pub shutdown: CancellationToken,
}

struct BaseLogger {
    name: String,
}

impl CallbackLogger for BaseLogger {
    fn callback_name(&self) -> &str {
        &self.name
    }
}

type ArmBuild = Arc<
    dyn Fn(
            CallbackName,
            &CatalogInputs,
            Option<&LoggerRuntime>,
            &SharedOtel,
            &mut Vec<Arc<dyn CallbackLogger>>,
        ) -> Result<Arc<dyn CallbackLogger>, CatalogError>
        + Send
        + Sync,
>;

pub struct LoggerCatalog {
    inputs: CatalogInputs,
    runtime: Option<Arc<LoggerRuntime>>,
    build: ArmBuild,
    built: Mutex<BTreeMap<CallbackName, Arc<dyn CallbackLogger>>>,
    // LiteLLM's _in_memory_loggers: every logger this catalog built, in build order
    in_memory: Mutex<Vec<Arc<dyn CallbackLogger>>>,
    shared: Mutex<Option<Arc<SharedOtel>>>,
}

impl LoggerCatalog {
    #[must_use]
    pub fn new(inputs: CatalogInputs) -> Self {
        Self {
            inputs,
            runtime: None,
            build: Arc::new(otel::dispatch::build_logger),
            built: Mutex::new(BTreeMap::new()),
            in_memory: Mutex::new(Vec::new()),
            shared: Mutex::new(None),
        }
    }

    // the OTel arms need B10007's runtime; every other name builds without one
    #[must_use]
    pub fn with_runtime(mut self, runtime: Arc<LoggerRuntime>) -> Self {
        self.runtime = Some(runtime);
        self
    }

    // clippy's same_name_method refuses this name while the type also implements the resolver trait
    pub fn resolve_logger(
        &self,
        name: &str,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        let Some(callback) = CallbackName::from_name(name) else {
            return Err(RegistryError::UnavailableAdapter {
                name: name.to_owned(),
            });
        };
        if let Some(logger) = self.existing(name) {
            return Ok(Some(logger));
        }
        let shared = self.shared();
        let built = {
            let mut in_memory = self
                .in_memory
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            (self.build)(
                callback,
                &self.inputs,
                self.runtime.as_deref(),
                &shared,
                &mut in_memory,
            )
        };
        let resolved = self.resolve_built(callback, built)?;
        if callback == CallbackName::Otel && resolved.is_some() {
            self.auto_start_phoenix();
        }
        Ok(resolved)
    }

    #[must_use]
    pub fn in_memory_loggers(&self) -> Vec<Arc<dyn CallbackLogger>> {
        self.in_memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // built on the first OTel arm: the snapshot never changes and the clients are shared by every arm
    fn shared(&self) -> Arc<SharedOtel> {
        let mut slot = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(shared) = slot.as_ref() {
            return Arc::clone(shared);
        }
        let shared = Arc::new(SharedOtel::new(&self.inputs));
        *slot = Some(Arc::clone(&shared));
        shared
    }

    fn auto_start_phoenix(&self) {
        if !phoenix_auto::configured(self.inputs.env.as_ref()) {
            return;
        }
        match self.resolve_logger(phoenix_auto::CALLBACK_NAME) {
            Ok(Some(logger)) => tracing::info!(
                logger = logger.callback_name(),
                "Auto-initialized Arize Phoenix logger alongside otel"
            ),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "Failed to auto-initialize Arize Phoenix logger");
            }
        }
    }

    #[must_use]
    pub fn existing(&self, name: &str) -> Option<Arc<dyn CallbackLogger>> {
        let callback = CallbackName::from_name(name)?;
        let built = self.built.lock().unwrap_or_else(PoisonError::into_inner);
        built.get(&callback).map(Arc::clone)
    }

    fn resolve_built(
        &self,
        callback: CallbackName,
        built: Result<Arc<dyn CallbackLogger>, CatalogError>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        match built {
            Ok(logger) => Ok(Some(self.store(callback, logger))),
            Err(CatalogError::Unavailable { name }) => {
                Err(RegistryError::UnavailableAdapter { name })
            }
            Err(CatalogError::Init { message }) => {
                tracing::error!("[Non-Blocking Error] Error initializing custom logger: {message}");
                Ok(None)
            }
        }
    }

    // another caller may have built this name meanwhile, and its logger stays the one handed out
    fn store(
        &self,
        callback: CallbackName,
        logger: Arc<dyn CallbackLogger>,
    ) -> Arc<dyn CallbackLogger> {
        let mut built = self.built.lock().unwrap_or_else(PoisonError::into_inner);
        let stored = Arc::clone(built.entry(callback).or_insert(logger));
        drop(built);
        stored
    }
}

impl LoggerResolver for LoggerCatalog {
    fn resolve(
        &self,
        name: &str,
        _params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError> {
        self.resolve_logger(name)
    }
}
