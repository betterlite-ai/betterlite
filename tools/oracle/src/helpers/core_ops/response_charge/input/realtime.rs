use std::collections::BTreeMap;

use betterlite_models::usage_sum::Usage as PricingUsage;

use super::{UsageInput, values};

// The typed pricing usage repeats the counters of the logging block, with no details of its own.
pub(crate) fn usage(fields: &UsageInput) -> PricingUsage {
    let usage = values::usage_block(fields);
    PricingUsage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        numeric_extras: BTreeMap::new(),
    }
}
