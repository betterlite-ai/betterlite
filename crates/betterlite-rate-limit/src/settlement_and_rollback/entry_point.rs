use crate::atomic_counter::payload::{
    IncrementAmounts,
    // ast-grep-ignore: no-long-names
    build_descriptor_atomic_payload as build_payload,
};
use crate::limiter_records::{DescriptorAtomicGroup, RateLimitDescriptor, RateLimitResponse};

use super::{AtomicSettlement, SettlementError};

mod tests;

impl AtomicSettlement<'_> {
    // ast-grep-ignore: no-long-names
    pub async fn atomic_check_and_increment_by_n(
        &self,
        descriptors: &[RateLimitDescriptor],
        increments: &[IncrementAmounts],
    ) -> Result<RateLimitResponse, SettlementError> {
        if descriptors.len() != increments.len() {
            return Err(SettlementError::DescriptorCount {
                descriptors: descriptors.len(),
                increments: increments.len(),
            });
        }
        let mut groups: Vec<DescriptorAtomicGroup> = Vec::with_capacity(descriptors.len());
        for (descriptor, increment_amounts) in descriptors.iter().zip(increments) {
            let group = build_payload(descriptor, *increment_amounts, self.default_window_size);
            if !group.0.is_empty() {
                groups.push(group);
            }
        }
        if groups.is_empty() {
            return Ok(RateLimitResponse {
                overall_code: "OK".to_owned(),
                statuses: Vec::new(),
                reservation_windows: None,
            });
        }
        self.dispatch(&groups).await
    }
}
