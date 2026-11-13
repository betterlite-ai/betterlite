use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GuardrailBaseError {
    #[error("'{hook}' is not a valid GuardrailEventHooks")]
    UnknownEventHook { hook: String },
    #[error("Event hook {hook} is not in the supported event hooks [{supported}]")]
    UnsupportedEventHook { hook: String, supported: String },
}
