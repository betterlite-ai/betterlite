use std::sync::{Mutex, PoisonError};

use betterlite_core::api::vector_file_provider::{
    CredentialLookup, CredentialLookupError, OperationKwargs,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) vector_store_id: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) kwargs: OperationKwargs,
    pub(crate) registry: Option<RegistryScript>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegistryScript {
    pub(crate) result: Option<OperationKwargs>,
    pub(crate) error: Option<ScriptedError>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ScriptedError {
    pub(crate) class_name: ScriptedClass,
    pub(crate) message: String,
}

#[derive(Debug, Deserialize)]
pub(crate) enum ScriptedClass {
    ValueError,
    RuntimeError,
}

pub(crate) struct RecordingLookup<'a> {
    pub(crate) script: &'a RegistryScript,
    pub(crate) calls: Mutex<Vec<String>>,
}

impl CredentialLookup for RecordingLookup<'_> {
    fn lookup_credentials(
        &self,
        vector_store_id: &str,
    ) -> Result<OperationKwargs, CredentialLookupError> {
        {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            calls.push(vector_store_id.to_owned());
        }
        if let Some(error) = &self.script.error {
            let class = match error.class_name {
                ScriptedClass::ValueError => "ValueError",
                ScriptedClass::RuntimeError => "RuntimeError",
            };
            let message = &error.message;
            return Err(CredentialLookupError::Unavailable {
                reason: format!("{class}: {message}"),
            });
        }
        Ok(self.script.result.clone().unwrap_or_default())
    }
}
