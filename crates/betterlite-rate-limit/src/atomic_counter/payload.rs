use serde::Deserialize;

use crate::limiter_records::{
    AtomicCounterMeta, DescriptorAtomicGroup, RateLimitDescriptor, WindowRateLimitType,
    create_rate_limit_keys,
};

use super::rate_limit_kind::status_rate_limit_type;

mod tests;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default)]
pub struct IncrementAmounts {
    pub requests: i64,
    pub tokens: i64,
}

#[derive(Clone, Copy)]
struct CounterSpec {
    rate_limit_type: WindowRateLimitType,
    limit: Option<i64>,
    increment: i64,
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn build_descriptor_atomic_payload(
    descriptor: &RateLimitDescriptor,
    increment_amounts: IncrementAmounts,
    default_window_size: i64,
) -> DescriptorAtomicGroup {
    let rate_limit = descriptor.rate_limit.as_ref();
    let window_size = rate_limit
        .and_then(|limits| limits.window_size)
        .unwrap_or(default_window_size);
    let window_key = format!("{{{}:{}}}:window", descriptor.key, descriptor.value);
    let specs = [
        CounterSpec {
            rate_limit_type: WindowRateLimitType::Requests,
            limit: rate_limit.and_then(|limits| limits.requests_per_unit),
            increment: increment_amounts.requests,
        },
        CounterSpec {
            rate_limit_type: WindowRateLimitType::Tokens,
            limit: rate_limit.and_then(|limits| limits.tokens_per_unit),
            increment: increment_amounts.tokens,
        },
    ];
    let mut group: DescriptorAtomicGroup = (Vec::new(), Vec::new(), Vec::new());
    for spec in specs {
        append_counter(&mut group, descriptor, &window_key, window_size, spec);
    }
    group
}

fn append_counter(
    group: &mut DescriptorAtomicGroup,
    descriptor: &RateLimitDescriptor,
    window_key: &str,
    window_size: i64,
    spec: CounterSpec,
) {
    let Some(current_limit) = spec.limit else {
        return;
    };
    if spec.increment < 0 {
        return;
    }
    let counter_key = create_rate_limit_keys(
        &descriptor.key,
        &descriptor.value,
        status_rate_limit_type(spec.rate_limit_type),
    );
    group.0.push(window_key.to_owned());
    group.0.push(counter_key.clone());
    group
        .1
        .extend([current_limit, spec.increment, window_size, window_size]);
    group.2.push(AtomicCounterMeta {
        descriptor_key: descriptor.key.clone(),
        descriptor_value: descriptor.value.clone(),
        current_limit,
        rate_limit_type: spec.rate_limit_type,
        window_key: window_key.to_owned(),
        counter_key,
        increment: spec.increment,
        ttl: window_size,
        window_size,
    });
}
