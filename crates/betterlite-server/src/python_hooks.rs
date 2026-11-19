use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::startup::process_environment;
use betterlite_pyhost::blocking_work::WorkTracker;
use betterlite_pyhost::call_error::CallError;
use betterlite_pyhost::hook_object::HookObject;
use betterlite_pyhost::host::{DEFAULT_SHUTDOWN_GRACE, HostConfig, HostError, PythonHost};
use betterlite_pyhost::instance_loader::{InstanceLoader, LoadError};
use betterlite_pyhost::remote_module::ModuleFetcher;
use betterlite_pyhost::return_check::ReturnCheckError;
use thiserror::Error;
use tokio::sync::OnceCell;
use tokio::task::{JoinError, JoinSet};

use self::bucket_fetcher::{BucketFetchError, BucketModuleFetcher};
use self::load::CustomProviderItem;
use self::slots::{HookSlot, ListKey, LoadedCallback, LoadedHook};

pub mod bucket_fetcher;
pub mod callbacks;
pub mod deadline;
pub mod load;
pub mod slots;

mod shutdown;
mod test_support;

// the pool size WorkTracker uses by default, which it does not expose
const HOOK_WORKERS: usize = 16;

pub struct PythonHooks {
    config_file_path: Option<PathBuf>,
    shutdown_grace: Duration,
    inner: OnceCell<HooksRuntime>,
}

pub struct HooksRuntime {
    pub host: PythonHost,
    pub tracker: Arc<WorkTracker>,
    pub loader: InstanceLoader<BucketFetchError>,
}

impl PythonHooks {
    #[must_use]
    pub fn new(config_file_path: Option<PathBuf>) -> Self {
        Self {
            config_file_path,
            shutdown_grace: DEFAULT_SHUTDOWN_GRACE,
            inner: OnceCell::new(),
        }
    }

    async fn start_runtime(&self) -> Result<HooksRuntime, HooksError> {
        let config = HostConfig {
            module_dirs: module_dirs(self.config_file_path.as_deref()),
            shutdown_grace: self.shutdown_grace,
        };
        let mut tasks = JoinSet::new();
        tasks.spawn_blocking(move || PythonHost::start(&config));
        let host = match tasks.join_next().await {
            Some(Ok(Ok(host))) => host,
            Some(Ok(Err(source))) => return Err(HooksError::Interpreter { source }),
            Some(Err(source)) => return Err(HooksError::InterpreterTask { source }),
            None => return Err(HooksError::InterpreterMissing),
        };
        let tracker = Arc::new(WorkTracker::new(HOOK_WORKERS, self.shutdown_grace));
        let fetcher: Arc<dyn ModuleFetcher<Error = BucketFetchError>> =
            Arc::new(BucketModuleFetcher::production(process_environment()));
        let loader = InstanceLoader::new(host.clone(), fetcher);
        Ok(HooksRuntime {
            host,
            tracker,
            loader,
        })
    }

    pub async fn load_hook(&self, slot: HookSlot, value: &str) -> Result<LoadedHook, HooksError> {
        let runtime = self.inner.get_or_try_init(|| self.start_runtime()).await?;
        let loaded = runtime
            .loader
            .load(&runtime.tracker, value, self.config_file_path.as_deref())
            .await
            .map_err(|source| HooksError::Load {
                slot: slot.as_str(),
                value: value.to_owned(),
                source: Box::new(source),
            })?;
        Ok(LoadedHook {
            slot,
            value: value.to_owned(),
            object: HookObject { loaded },
            host: runtime.host.clone(),
            tracker: Arc::clone(&runtime.tracker),
        })
    }

    pub async fn load_callback(
        &self,
        list_key: ListKey,
        entry: &str,
    ) -> Result<LoadedCallback, HooksError> {
        if !entry.contains('.') {
            // LiteLLM's own callbacks are plain names like "langfuse" and never Python objects
            return Ok(LoadedCallback::Named(entry.to_owned()));
        }
        let hook = self.load_hook(HookSlot::Callback(list_key), entry).await?;
        if list_key == ListKey::Callbacks {
            callbacks::check_dispatchable(&hook, entry)?;
        }
        Ok(LoadedCallback::Loaded(hook))
    }

    pub async fn load_post_call_rules(&self, value: &str) -> Result<Vec<LoadedHook>, HooksError> {
        self.load_hook(HookSlot::PostCallRules, value)
            .await
            .map(|hook| vec![hook])
    }

    pub async fn load_custom_providers(
        &self,
        items: &[CustomProviderItem],
    ) -> Result<Vec<(String, HookObject)>, HooksError> {
        let mut providers = Vec::with_capacity(items.len());
        for item in items {
            let hook = self
                .load_hook(HookSlot::CustomProvider, &item.custom_handler)
                .await?;
            providers.push((item.provider.clone(), hook.object));
        }
        Ok(providers)
    }

    pub async fn load_custom_validate(&self, value: &str) -> Result<LoadedHook, HooksError> {
        let hook = self.load_hook(HookSlot::CustomValidate, value).await?;
        load::require_literal_true_return(&hook, value)?;
        Ok(hook)
    }

    pub async fn run_startup_hooks(&self, specs: &[String]) -> Result<(), HooksError> {
        for spec in specs {
            load::run_startup_hook(self, spec).await?;
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        if let Some(runtime) = self.inner.get() {
            runtime.tracker.shutdown().await;
        }
    }
}

// LiteLLM inserts the folder that holds config.yaml at the front of sys.path
fn module_dirs(config_file_path: Option<&Path>) -> Vec<PathBuf> {
    config_file_path.map_or_else(Vec::new, |path| {
        vec![path.parent().unwrap_or(path).to_path_buf()]
    })
}

#[derive(Debug, Error)]
pub enum HooksError {
    #[error("the Python interpreter could not start: {source}")]
    Interpreter {
        #[source]
        source: HostError,
    },
    #[error("starting the Python interpreter failed: {source}")]
    InterpreterTask {
        #[source]
        source: JoinError,
    },
    #[error("starting the Python interpreter returned no result")]
    InterpreterMissing,
    #[error("could not load the {slot} hook {value:?}: {source}")]
    Load {
        slot: &'static str,
        value: String,
        #[source]
        source: Box<LoadError<BucketFetchError>>,
    },
    #[error(
        "litellm_settings.callbacks entry '{entry}' resolved to the class {module}.{qualname}, which is neither a CustomLogger instance nor a callable, so the proxy would never run it. Point it at an instance instead, e.g. add `proxy_handler_instance = {name}()` to {module_path} and set `callbacks: [\"{module_path}.proxy_handler_instance\"]`."
    )]
    CallbackIsClass {
        entry: String,
        module: String,
        qualname: String,
        name: String,
        module_path: String,
    },
    #[error(
        "litellm_settings.callbacks entry '{entry}' resolved to {type_name} {repr}, which is neither a CustomLogger instance nor a callable, so the proxy would never run it."
    )]
    CallbackNotDispatchable {
        entry: String,
        type_name: String,
        repr: String,
    },
    #[error("custom_validate {value:?} is not usable: {source}")]
    Validate {
        value: String,
        #[source]
        source: Box<ReturnCheckError>,
    },
    #[error(transparent)]
    Call(#[from] CallError),
    #[error("Invalid hook spec '{spec}': expected format is 'module.path:function_name'")]
    StartupSpec { spec: String },
    #[error("Worker startup hook '{spec}' failed: {message}")]
    Startup {
        spec: String,
        message: String,
        #[source]
        source: Box<load::StartupFailure>,
    },
}
