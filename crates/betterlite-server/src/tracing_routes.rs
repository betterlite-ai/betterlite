pub mod access;
pub mod otlp;
pub mod query;
pub mod receiver;
pub mod router;
mod tests;

use std::sync::Arc;

use secrecy::SecretString;

use crate::tracing_routes::access::TraceClock;
use crate::tracing_routes::receiver::TraceRuntime;

#[derive(Clone)]
pub struct TraceRoutesState {
    pub receiver: Option<Arc<dyn TraceRuntime>>,
    pub master_key: Option<SecretString>,
    pub clock: Arc<dyn TraceClock>,
}
