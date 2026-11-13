use std::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupFailure {
    MissingSetting,
    Storage,
    Runtime,
    InvalidValue,
    Unexpected,
}

// the four exception classes _start_receiver warns about; an adapter classifies its own typed error
pub trait ReceiverFailure: Error + Send + Sync + 'static {
    fn startup_failure(&self) -> StartupFailure;
}

#[derive(Debug, thiserror::Error)]
pub enum TraceLifecycleError<ReceiverError, LoggerError> {
    #[error("agent tracing receiver failed")]
    Receiver {
        #[source]
        source: ReceiverError,
    },
    #[error("the agent tracing spend logger could not be built")]
    SpendLogger {
        #[source]
        source: LoggerError,
    },
}
