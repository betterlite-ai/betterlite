use std::collections::BTreeMap;

use crate::limiter_records::{
    ParallelRequestGauge, RateLimitDescriptor, RateLimitType, WindowKeyMetadata,
    create_rate_limit_keys,
};

use super::WindowedAdmission;

#[derive(Default)]
pub struct CollectedWindowedKeys {
    pub keys_to_fetch: Vec<String>,
    pub key_metadata: BTreeMap<String, WindowKeyMetadata>,
    pub gauges: Vec<ParallelRequestGauge>,
}

impl WindowedAdmission {
    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn collect_windowed_keys_and_gauges(
        &self,
        descriptors: &[RateLimitDescriptor],
        skip_tpm_check: bool,
    ) -> CollectedWindowedKeys {
        let mut collected = CollectedWindowedKeys::default();
        for descriptor in descriptors {
            collect_descriptor(&mut collected, descriptor, skip_tpm_check, self.window_size);
        }
        collected
    }
}

struct DescriptorLimits {
    requests: Option<i64>,
    tokens: Option<i64>,
    max_parallel: Option<i64>,
    window_size: i64,
}

fn descriptor_limits(
    descriptor: &RateLimitDescriptor,
    skip_tpm_check: bool,
    handler_window_size: i64,
) -> DescriptorLimits {
    let limit = descriptor.rate_limit.as_ref();
    let tokens = if skip_tpm_check {
        None
    } else {
        limit.and_then(|limit| limit.tokens_per_unit)
    };
    // LiteLLM reads the descriptor's window_size with `or`, so a 0 falls back to the handler's
    let window_size = limit
        .and_then(|limit| limit.window_size)
        .filter(|size| *size != 0)
        .unwrap_or(handler_window_size);
    DescriptorLimits {
        requests: limit.and_then(|limit| limit.requests_per_unit),
        tokens,
        max_parallel: limit.and_then(|limit| limit.max_parallel_requests),
        window_size,
    }
}

fn collect_descriptor(
    collected: &mut CollectedWindowedKeys,
    descriptor: &RateLimitDescriptor,
    skip_tpm_check: bool,
    handler_window_size: i64,
) {
    let limits = descriptor_limits(descriptor, skip_tpm_check, handler_window_size);
    let (key, value) = (descriptor.key.as_str(), descriptor.value.as_str());
    let window_key = format!("{{{key}:{value}}}:window");
    collect_gauge(collected, descriptor, limits.max_parallel);
    collect_window_keys(collected, descriptor, &window_key, &limits);
    if limits.requests.is_some() || limits.tokens.is_some() {
        collected.key_metadata.insert(
            window_key,
            WindowKeyMetadata {
                requests_limit: limits.requests,
                tokens_limit: limits.tokens,
                window_size: limits.window_size,
                descriptor_key: descriptor.key.clone(),
                descriptor_value: descriptor.value.clone(),
            },
        );
    }
}

fn collect_gauge(
    collected: &mut CollectedWindowedKeys,
    descriptor: &RateLimitDescriptor,
    max_parallel: Option<i64>,
) {
    let Some(limit) = max_parallel else {
        return;
    };
    collected.gauges.push(ParallelRequestGauge {
        counter_key: create_rate_limit_keys(
            &descriptor.key,
            &descriptor.value,
            RateLimitType::MaxParallelRequests,
        ),
        limit,
        descriptor_key: descriptor.key.clone(),
    });
}

fn collect_window_keys(
    collected: &mut CollectedWindowedKeys,
    descriptor: &RateLimitDescriptor,
    window_key: &str,
    limits: &DescriptorLimits,
) {
    if limits.requests.is_some() {
        push_counter_key(
            &mut collected.keys_to_fetch,
            window_key,
            descriptor,
            RateLimitType::Requests,
        );
    }
    if limits.tokens.is_some() {
        push_counter_key(
            &mut collected.keys_to_fetch,
            window_key,
            descriptor,
            RateLimitType::Tokens,
        );
    }
}

fn push_counter_key(
    keys_to_fetch: &mut Vec<String>,
    window_key: &str,
    descriptor: &RateLimitDescriptor,
    rate_limit_type: RateLimitType,
) {
    keys_to_fetch.push(window_key.to_owned());
    keys_to_fetch.push(create_rate_limit_keys(
        &descriptor.key,
        &descriptor.value,
        rate_limit_type,
    ));
}

mod tests;
