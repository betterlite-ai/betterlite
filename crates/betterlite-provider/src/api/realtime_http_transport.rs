use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::realtime_contracts::config::BaseRealtimeHttpConfig;

use client::{RealtimeClientSource, SuppliedClient};

pub mod calls;
pub mod client;
pub mod decode;
pub mod error;
pub(crate) mod send;
pub mod session;
#[cfg(test)]
mod tests;

// The HTTP client the handlers reach, the provider config that names the endpoint, and the API body cap the SDP decoder enforces
pub struct TransportDeps<'deps, C: BaseRealtimeHttpConfig> {
    pub config: Option<&'deps C>,
    pub source: &'deps dyn RealtimeClientSource,
    pub supplied: Option<SuppliedClient<'deps>>,
    pub limits: ApiLimits,
}
