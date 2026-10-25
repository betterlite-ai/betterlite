use thiserror::Error;

#[derive(Debug, Error)]
#[error("harness error: {0}")]
pub struct HarnessError(pub String);

macro_rules! harness_error {
    ($name:ident, $message:literal) => {
        #[derive(Debug, Error)]
        #[error($message)]
        pub struct $name(pub String);
    };
}

harness_error!(CapabilityUnsupported, "capability unsupported: {0}");
harness_error!(OptionsMismatch, "harness options mismatch: {0}");
harness_error!(HarnessInstallFailed, "harness installation failed: {0}");
harness_error!(SandboxError, "sandbox error: {0}");
harness_error!(SessionClosed, "session closed: {0}");
harness_error!(StateIncompatible, "state incompatible: {0}");

#[derive(Debug, Error)]
#[error("output is invalid: {message}")]
pub struct OutputInvalid {
    pub message: String,
    pub raw: String,
    pub result: Option<crate::types::Result>,
}

impl OutputInvalid {
    pub fn new(message: String, raw: String, result: Option<crate::types::Result>) -> Self {
        let error = Self {
            message,
            raw,
            result,
        };
        error
    }
}
