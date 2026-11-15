use crate::atomic_counter::payload::{
    IncrementAmounts,
    // ast-grep-ignore: no-long-names
    build_descriptor_atomic_payload as build_payload,
};
use crate::limiter_records::{DescriptorAtomicGroup, RateLimitDescriptor};

use super::SettlementError;

mod tests;

pub(super) fn descriptor_groups(
    descriptors: &[RateLimitDescriptor],
    increments: &[IncrementAmounts],
    default_window_size: i64,
) -> Result<Vec<DescriptorAtomicGroup>, SettlementError> {
    if descriptors.len() != increments.len() {
        return Err(SettlementError::DescriptorCount {
            descriptors: descriptors.len(),
            increments: increments.len(),
        });
    }
    let mut groups: Vec<DescriptorAtomicGroup> = Vec::with_capacity(descriptors.len());
    for (descriptor, increment_amounts) in descriptors.iter().zip(increments) {
        let group = build_payload(descriptor, *increment_amounts, default_window_size);
        if !group.0.is_empty() {
            groups.push(group);
        }
    }
    Ok(groups)
}
