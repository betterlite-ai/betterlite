// The only failure the runtime reports: the injected port could not build the callback LiteLLM's
// OpenTelemetryV2 constructor would have raised on; preset failures stay inside maybe_construct's fallback
#[derive(Debug, thiserror::Error)]
pub enum LoggerRuntimeError {
    #[error("the OTel V2 logger constructor failed: {message}")]
    Constructor { message: String },
}
