use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_logging::callback_logger::resolver::LoggerResolver;
use betterlite_logging::dynamic_callbacks::DynamicCallback;

use super::hooks::DeploymentHook;

pub mod custom_logger;

use custom_logger::is_same_registered_logger;

// LiteLLM keeps these five lists as module globals; betterlite passes the caller's copy through every call
#[derive(Debug, Default)]
pub struct CallbackLists {
    pub input: Vec<DynamicCallback>,
    pub success: Vec<DynamicCallback>,
    pub async_success: Vec<DynamicCallback>,
    pub failure: Vec<DynamicCallback>,
    pub async_failure: Vec<DynamicCallback>,
}

// The catalog of custom logger names and the resolver that builds them, both injected by the caller
pub struct CustomLoggers<'call> {
    pub known: &'call dyn Fn(&str) -> bool,
    pub resolver: &'call dyn LoggerResolver,
}

impl CallbackLists {
    pub fn add_input(&mut self, callback: &DynamicCallback) {
        if !self.input.contains(callback) {
            self.input.push(callback.clone());
        }
    }

    // The manager routes dynamodb and openmeter to the async list even when the caller named them for success
    pub fn add_success(&mut self, callback: &DynamicCallback) {
        if let DynamicCallback::Name(name) = callback
            && matches!(name.as_str(), "dynamodb" | "openmeter")
        {
            self.add_async_success(callback);
            return;
        }
        if !registered(&self.success, callback) {
            self.success.push(callback.clone());
        }
    }

    pub fn add_failure(&mut self, callback: &DynamicCallback) {
        if !registered(&self.failure, callback) {
            self.failure.push(callback.clone());
        }
    }

    pub fn add_async_success(&mut self, callback: &DynamicCallback) {
        if !registered(&self.async_success, callback) {
            self.async_success.push(callback.clone());
        }
    }

    pub fn add_async_failure(&mut self, callback: &DynamicCallback) {
        if !registered(&self.async_failure, callback) {
            self.async_failure.push(callback.clone());
        }
    }
}

fn registered(callbacks: &[DynamicCallback], callback: &DynamicCallback) -> bool {
    callbacks
        .iter()
        .any(|existing| existing == callback || is_same_registered_logger(existing, callback))
}

#[must_use]
pub fn get_dynamic_callbacks(
    global: &[Arc<dyn DeploymentHook>],
    dynamic: Option<&[String]>,
) -> Vec<DynamicCallback> {
    let mut callbacks: Vec<DynamicCallback> = global
        .iter()
        .map(|hook| {
            let shared = Arc::clone(hook);
            let logger: Arc<dyn CallbackLogger> = shared;
            DynamicCallback::Logger(logger)
        })
        .collect();
    for name in dynamic.unwrap_or_default() {
        callbacks.push(DynamicCallback::Name(name.clone()));
    }
    callbacks
}
