use std::sync::Arc;

use betterlite_auth::resolved_key::auth::ParentOtelSpan;

use crate::atomic_counter::payload::IncrementAmounts;
use crate::fail_closed::RateLimitUnverifiableError;
use crate::limiter_assembly::RateLimiterAssembly;
use crate::limiter_records::{RateLimitDescriptor, RateLimitResponse};
use crate::settlement_and_rollback::SettlementError;
use crate::windowed_admission::{AdmissionRequest, WindowedAdmissionError};

mod tests;

// The two limiter calls the dynamic hook makes, behind a port so a test can answer without Redis
#[async_trait::async_trait]
pub trait AdmissionLimiter: Send + Sync {
    fn window_size(&self) -> i64;

    // ast-grep-ignore: no-long-names
    async fn atomic_check_and_increment_by_n(
        &self,
        descriptors: &[RateLimitDescriptor],
        increments: &[IncrementAmounts],
        parent_span: Option<ParentOtelSpan>,
    ) -> Result<RateLimitResponse, AdmissionLimiterError>;

    async fn should_rate_limit(
        &self,
        descriptors: &[RateLimitDescriptor],
        parent_span: Option<ParentOtelSpan>,
        read_only: bool,
    ) -> Result<RateLimitResponse, AdmissionLimiterError>;
}

// every way the port fails: the fail-closed 503 keeps its own variant, and the hook finds it wherever a wrapping error carries it
#[derive(Debug, thiserror::Error)]
pub enum AdmissionLimiterError {
    #[error(transparent)]
    Unverifiable(#[from] RateLimitUnverifiableError),
    #[error(transparent)]
    Settlement(#[from] SettlementError),
    #[error(transparent)]
    Windowed(#[from] WindowedAdmissionError),
}

impl AdmissionLimiterError {
    // a settlement that carried the fail-closed 503 up is flattened, so the caller sees one Unverifiable place
    #[must_use]
    pub fn from_settlement(error: SettlementError) -> Self {
        match error {
            SettlementError::Unverifiable(inner) => Self::Unverifiable(inner),
            other @ (SettlementError::DescriptorCount { .. }
            | SettlementError::Reply(_)
            | SettlementError::Response(_)
            | SettlementError::AtomicCounter(_)
            | SettlementError::WindowCounter(_)
            | SettlementError::ClockBeforeEpoch(_)
            | SettlementError::ClockOverflow { .. }
            | SettlementError::UnpairedRefundKeys { .. }
            | SettlementError::RefundPosition { .. }) => Self::Settlement(other),
        }
    }
}

// B5660's assembly behind the port: its window size for retry-after, B3034's atomic entry point, and B3036's windowed check
pub struct AssemblyLimiter {
    pub assembly: Arc<RateLimiterAssembly>,
}

#[async_trait::async_trait]
impl AdmissionLimiter for AssemblyLimiter {
    fn window_size(&self) -> i64 {
        self.assembly.settings.window_size
    }

    // ast-grep-ignore: no-long-names
    async fn atomic_check_and_increment_by_n(
        &self,
        descriptors: &[RateLimitDescriptor],
        increments: &[IncrementAmounts],
        _parent_span: Option<ParentOtelSpan>,
    ) -> Result<RateLimitResponse, AdmissionLimiterError> {
        self.assembly
            .settlement()
            .atomic_check_and_increment_by_n(descriptors, increments)
            .await
            .map_err(AdmissionLimiterError::from_settlement)
    }

    async fn should_rate_limit(
        &self,
        descriptors: &[RateLimitDescriptor],
        parent_span: Option<ParentOtelSpan>,
        read_only: bool,
    ) -> Result<RateLimitResponse, AdmissionLimiterError> {
        let request = AdmissionRequest {
            descriptors: descriptors.to_vec(),
            parent_otel_span: parent_span,
            read_only,
            skip_tpm_check: false,
            parallel_slot_id: None,
        };
        self.assembly
            .admission
            .should_rate_limit(&request)
            .await
            .map_err(AdmissionLimiterError::from)
    }
}
