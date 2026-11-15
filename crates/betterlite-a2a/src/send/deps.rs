use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::tls::TlsPolicy;
use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_state::correlation::{CorrelationContext, CorrelationSettings};
use betterlite_redact::secret::Redactor;
use betterlite_tokenizer::count::Counter;

// Everything the native A2A path reads from the process: the HTTP pool, env, TLS and the token counter
pub struct NativeDeps<'deps> {
    pub http: &'deps ProxiedClient,
    pub environment: &'deps (dyn EnvironmentLookup + Sync),
    pub tls: &'deps TlsPolicy,
    pub counter: &'deps Counter,
    pub trace_ids: &'deps (dyn Fn() -> String + Sync),
}

// The streaming path also builds a Logging object and wraps the answer in the accounting iterator
pub struct StreamingDeps<'deps, Clock, Dispatch> {
    pub native: NativeDeps<'deps>,
    pub clock: Clock,
    pub monotonic: &'deps mut (dyn FnMut() -> Duration + Send),
    pub correlation: &'deps mut CorrelationContext,
    pub correlation_settings: CorrelationSettings,
    pub redactor: &'deps Redactor,
    pub dispatch: Dispatch,
    pub max_retained_bytes: usize,
}

pub struct BridgeDeps<'call, C, P, S> {
    pub completion: &'call C,
    pub providers: &'call P,
    pub source: &'call mut S,
}
