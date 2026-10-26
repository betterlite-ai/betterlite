use std::collections::BTreeMap;

pub const OUTPUT: &str = "output";
pub const INPUT: &str = "input";
pub const TOTAL: &str = "total";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseObject {
    Usage(Usage),
    WithUsage(Option<Usage>),
    Mapping(Usage),
}

pub fn get_rate_limit_type(settings: &BTreeMap<String, String>) -> &'static str {
    match settings.get("token_rate_limit_type").map(String::as_str) {
        Some(OUTPUT) => OUTPUT,
        Some(INPUT) => INPUT,
        _ => TOTAL,
    }
}

#[must_use]
pub const fn response_usage(response: &ResponseObject) -> Option<&Usage> {
    match response {
        ResponseObject::Usage(usage) | ResponseObject::Mapping(usage) => Some(usage),
        ResponseObject::WithUsage(usage) => usage.as_ref(),
    }
}

#[must_use]
pub fn get_total_tokens_from_usage(source: Option<&Usage>, rate_limit_type: &str) -> u64 {
    let Some(usage) = source else { return 0 };
    let count = match rate_limit_type {
        OUTPUT => usage.output_tokens.max(usage.completion_tokens),
        INPUT => usage.input_tokens.max(usage.prompt_tokens),
        _ => usage.total_tokens,
    };
    if rate_limit_type == INPUT || rate_limit_type == TOTAL {
        let cached = usage.cached_tokens.or(usage.cache_read_input_tokens);
        count.saturating_sub(cached.unwrap_or(0))
    } else {
        count
    }
}

#[must_use]
pub const fn aggregate_only_total_tokens(source: Option<&Usage>) -> u64 {
    let Some(usage) = source else { return 0 };
    if usage.prompt_tokens > 0
        || usage.completion_tokens > 0
        || usage.input_tokens > 0
        || usage.output_tokens > 0
    {
        0
    } else {
        usage.total_tokens
    }
}

#[must_use]
pub fn resolve_rerank_token_usage(
    tokens: Option<(u64, u64)>,
    billed_total: Option<u64>,
) -> Option<(u64, u64, bool)> {
    if let Some((input, output)) = tokens
        && (input > 0 || output > 0)
    {
        return Some((input, output, true));
    }
    billed_total
        .filter(|total| *total > 0)
        .map(|total| (total, 0, true))
}

#[must_use]
pub fn reconcile_io_usage(response: &ResponseObject) -> (u64, u64, bool) {
    let Some(usage) = response_usage(response) else {
        return (0, 0, false);
    };
    let input = usage.input_tokens.max(usage.prompt_tokens);
    let output = usage.output_tokens.max(usage.completion_tokens);
    if input == 0 && output == 0 {
        return (0, 0, false);
    }
    let cached = usage.cached_tokens.or(usage.cache_read_input_tokens);
    (input.saturating_sub(cached.unwrap_or(0)), output, true)
}

#[must_use]
pub fn recovered_partial_usage_tokens(
    source: Option<&Usage>,
    rate_limit_type: &str,
) -> (u64, u64, u64) {
    let Some(usage) = source else {
        return (0, 0, 0);
    };
    if usage.completion_tokens == 0 && usage.output_tokens == 0 {
        return (0, 0, 0);
    }
    let (input, output, _) = reconcile_io_usage(&ResponseObject::Usage(usage.clone()));
    (
        get_total_tokens_from_usage(Some(usage), rate_limit_type),
        input,
        output,
    )
}

mod tests;
