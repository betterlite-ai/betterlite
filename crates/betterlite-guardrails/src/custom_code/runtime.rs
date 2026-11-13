use std::future::Future;
use std::pin::Pin;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use thiserror::Error;

use crate::custom_code_execution::execution::error::SandboxError;
use crate::custom_code_execution::sandbox::CompiledProgram;
use crate::custom_code_execution::sandbox::globals::SandboxGlobals;
use crate::generic_guardrail_contracts::GuardrailInputType;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct RuntimeFailure {
    pub message: String,
}

#[derive(Debug, Error)]
pub enum LoadFailure {
    #[error(
        "Custom code must define an 'apply_guardrail' function. Expected signature: apply_guardrail(inputs, request_data, input_type)"
    )]
    MissingFunction,
    #[error("'apply_guardrail' must be a callable function")]
    NotCallable,
    #[error(transparent)]
    Runtime(RuntimeFailure),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallArguments {
    pub inputs: GuardrailValue,
    pub request_data: GuardrailValue,
    pub input_type: GuardrailInputType,
}

pub type PendingCall =
    Pin<Box<dyn Future<Output = Result<GuardrailValue, SandboxError<RuntimeFailure>>> + Send>>;

pub enum CallOutput {
    Value(GuardrailValue),
    Pending(PendingCall),
}

// The runtime that executes sandboxed source is supplied by the caller; betterlite has no Python.
pub trait ModuleHost: Send + Sync {
    fn load(
        &self,
        program: &CompiledProgram,
        globals: &SandboxGlobals,
    ) -> Result<Box<dyn LoadedModule>, SandboxError<LoadFailure>>;
}

pub trait LoadedModule: Send + Sync {
    fn call(&self, arguments: CallArguments) -> Result<CallOutput, SandboxError<RuntimeFailure>>;
}

#[must_use]
pub const fn input_type_name(input_type: GuardrailInputType) -> &'static str {
    match input_type {
        GuardrailInputType::Request => "request",
        GuardrailInputType::Response => "response",
    }
}
