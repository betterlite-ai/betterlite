use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_pyhost::blocking_work::WorkError;
use betterlite_pyhost::call_error::{CallError, PythonError};
use betterlite_pyhost::callable_handle::CallableHandle;
use betterlite_pyhost::hook_object::HookObject;
use betterlite_pyhost::return_check::check_literal_true_return;
use pyo3::types::PyAnyMethods as _;
use pyo3::{Bound, Py, PyAny, Python};

use super::deadline::LOAD_DEADLINE;
use super::slots::{HookSlot, LoadedHook};
use super::{HooksError, HooksRuntime, PythonHooks};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomProviderItem {
    pub provider: String,
    pub custom_handler: String,
}

impl PythonHooks {
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
        require_literal_true_return(&hook, value)?;
        Ok(hook)
    }

    pub async fn run_startup_hooks(&self, specs: &[String]) -> Result<(), HooksError> {
        for spec in specs {
            self.run_startup_hook(spec).await?;
        }
        Ok(())
    }

    async fn run_startup_hook(&self, spec: &str) -> Result<(), HooksError> {
        let Some((module_path, function_name)) = spec.rsplit_once(':') else {
            return Err(HooksError::StartupSpec {
                spec: spec.to_owned(),
            });
        };
        let runtime = self.inner.get_or_try_init(|| self.start_runtime()).await?;
        match startup_call(runtime, module_path, function_name).await {
            Ok(()) => {
                tracing::info!("Worker startup hook '{spec}' executed successfully");
                Ok(())
            }
            Err(source) => {
                let message = startup_error_text(&source);
                tracing::error!("Worker startup hook '{spec}' failed: {message}");
                Err(HooksError::Startup {
                    spec: spec.to_owned(),
                    message,
                    source: Box::new(source),
                })
            }
        }
    }
}

fn require_literal_true_return(hook: &LoadedHook, value: &str) -> Result<(), HooksError> {
    let handle = hook.object.loaded.handle(&hook.host)?;
    check_literal_true_return(&hook.host, Some(&handle)).map_err(|source| {
        HooksError::Validate {
            value: value.to_owned(),
            source: Box::new(source),
        }
    })?;
    Ok(())
}

// LiteLLM logs str(exception), which is the Python message without the exception type
fn startup_error_text(failure: &StartupFailure) -> String {
    match failure {
        StartupFailure::Import(source) | StartupFailure::Call(CallError::Python(source)) => {
            source.message.clone()
        }
        other @ (StartupFailure::Call(_) | StartupFailure::Work(_)) => other.to_string(),
    }
}

async fn startup_call(
    runtime: &HooksRuntime,
    module_path: &str,
    function_name: &str,
) -> Result<(), StartupFailure> {
    let function = import_function(runtime, module_path, function_name).await?;
    let handle = CallableHandle::new(&runtime.host, function)?;
    handle
        .call_raw(
            &runtime.host,
            &runtime.tracker,
            LOAD_DEADLINE,
            Vec::new(),
            BTreeMap::new(),
        )
        .await?;
    Ok(())
}

async fn import_function(
    runtime: &HooksRuntime,
    module_path: &str,
    function_name: &str,
) -> Result<Py<PyAny>, StartupFailure> {
    let module = module_path.to_owned();
    let function = function_name.to_owned();
    runtime
        .tracker
        .run(&runtime.host, LOAD_DEADLINE, move |py| {
            import_in_interpreter(py, &module, &function)
        })
        .await
        .map_err(StartupFailure::Work)?
        .map_err(StartupFailure::Import)
}

fn import_in_interpreter(
    py: Python<'_>,
    module_path: &str,
    function_name: &str,
) -> Result<Py<PyAny>, PythonError> {
    py.import(module_path)
        .and_then(|module| module.getattr(function_name))
        .map(Bound::unbind)
        .map_err(|error| PythonError::from_pyerr(py, &error))
}

#[derive(Debug, thiserror::Error)]
pub enum StartupFailure {
    #[error(transparent)]
    Import(#[from] PythonError),
    #[error(transparent)]
    Work(#[from] WorkError),
    #[error(transparent)]
    Call(#[from] CallError),
}

mod tests;
