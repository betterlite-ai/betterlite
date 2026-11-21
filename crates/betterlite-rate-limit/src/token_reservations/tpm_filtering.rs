use crate::limiter_records::{
    PROJECT_ITPM_DESCRIPTOR_KEY, PROJECT_OTPM_DESCRIPTOR_KEY, RateLimitDescriptor,
    RateLimitDescriptorRateLimit,
};

mod tests;

pub(crate) fn tpm_descriptors(descriptors: &[RateLimitDescriptor]) -> Vec<RateLimitDescriptor> {
    let mut filtered = Vec::new();
    for descriptor in descriptors {
        if descriptor.key == PROJECT_ITPM_DESCRIPTOR_KEY
            || descriptor.key == PROJECT_OTPM_DESCRIPTOR_KEY
        {
            continue;
        }
        let Some(tokens_per_unit) = descriptor
            .rate_limit
            .as_ref()
            .and_then(|limits| limits.tokens_per_unit)
        else {
            continue;
        };
        filtered.push(RateLimitDescriptor {
            key: descriptor.key.clone(),
            value: descriptor.value.clone(),
            rate_limit: Some(RateLimitDescriptorRateLimit {
                requests_per_unit: None,
                tokens_per_unit: Some(tokens_per_unit),
                max_parallel_requests: None,
                window_size: descriptor
                    .rate_limit
                    .as_ref()
                    .and_then(|limits| limits.window_size),
            }),
        });
    }
    filtered
}
