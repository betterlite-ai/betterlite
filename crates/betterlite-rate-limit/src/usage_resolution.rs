use serde::Deserialize;

pub const OUTPUT: &str = "output";
pub const INPUT: &str = "input";
pub const TOTAL: &str = "total";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TokenRateLimitType {
    Output,
    Input,
    #[default]
    Total,
}

impl TokenRateLimitType {
    #[must_use]
    pub fn from_config(value: Option<&str>) -> Self {
        match value {
            Some(OUTPUT) => Self::Output,
            Some(INPUT) => Self::Input,
            _ => Self::Total,
        }
    }
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Output => OUTPUT,
            Self::Input => INPUT,
            Self::Total => TOTAL,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct TokenDetails {
    #[serde(default)]
    pub cached_tokens: u64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub prompt_tokens_details: Option<TokenDetails>,
    #[serde(default)]
    pub input_tokens_details: Option<TokenDetails>,
    #[serde(default)]
    pub cached_tokens: Option<u64>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct ResponseAPIUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub input_tokens_details: Option<TokenDetails>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseObject {
    Usage(Usage),
    ResponseApi(ResponseAPIUsage),
    WithUsage(Option<Usage>),
    Mapping(Usage),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResponseUsage<'usage> {
    Usage(&'usage Usage),
    ResponseApi(&'usage ResponseAPIUsage),
    Mapping(&'usage Usage),
}

#[must_use]
pub const fn get_rate_limit_type(rate_limit_type: TokenRateLimitType) -> &'static str {
    match rate_limit_type {
        TokenRateLimitType::Output => OUTPUT,
        TokenRateLimitType::Input => INPUT,
        TokenRateLimitType::Total => TOTAL,
    }
}

#[must_use]
pub fn response_usage(response: &ResponseObject) -> Option<ResponseUsage<'_>> {
    match response {
        ResponseObject::Usage(usage) => Some(ResponseUsage::Usage(usage)),
        ResponseObject::Mapping(usage) => Some(ResponseUsage::Mapping(usage)),
        ResponseObject::ResponseApi(usage) => Some(ResponseUsage::ResponseApi(usage)),
        ResponseObject::WithUsage(usage) => usage.as_ref().map(ResponseUsage::Usage),
    }
}

fn cached_tokens(usage: &Usage) -> u64 {
    let cached = usage
        .cached_tokens
        .or_else(|| {
            usage
                .prompt_tokens_details
                .as_ref()
                .map(|details| details.cached_tokens)
        })
        .or_else(|| {
            usage
                .input_tokens_details
                .as_ref()
                .map(|details| details.cached_tokens)
        })
        .or(usage.cache_read_input_tokens)
        .unwrap_or(0);
    if cached == 0 { 0 } else { cached }
}

fn cached_response_api_tokens(usage: &ResponseAPIUsage) -> u64 {
    let cached = usage
        .input_tokens_details
        .as_ref()
        .map_or(0, |details| details.cached_tokens);
    if cached == 0 { 0 } else { cached }
}

#[must_use]
pub fn get_total_tokens_from_usage(
    source: Option<ResponseUsage<'_>>,
    rate_limit_type: TokenRateLimitType,
) -> u64 {
    let Some(usage_source) = source else { return 0 };
    let (count, cached) = match usage_source {
        ResponseUsage::Usage(usage) | ResponseUsage::Mapping(usage) => {
            let count = match rate_limit_type {
                TokenRateLimitType::Output => usage.completion_tokens,
                TokenRateLimitType::Input => usage.prompt_tokens,
                TokenRateLimitType::Total => usage.total_tokens,
            };
            (count, cached_tokens(usage))
        }
        ResponseUsage::ResponseApi(_) => (0, 0),
    };
    match rate_limit_type {
        TokenRateLimitType::Output => count,
        TokenRateLimitType::Input | TokenRateLimitType::Total => count.saturating_sub(cached),
    }
}

#[must_use]
pub const fn aggregate_only_total_tokens(source: Option<ResponseUsage<'_>>) -> u64 {
    let Some(usage_source) = source else { return 0 };
    match usage_source {
        ResponseUsage::Usage(usage) | ResponseUsage::Mapping(usage) => {
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
        ResponseUsage::ResponseApi(usage) => {
            if usage.input_tokens > 0 || usage.output_tokens > 0 {
                0
            } else {
                usage.total_tokens
            }
        }
    }
}

#[must_use]
pub const fn resolve_rerank_token_usage(
    tokens: Option<(u64, u64)>,
    billed_total: Option<u64>,
) -> Option<(u64, u64, bool)> {
    if let Some((input, output)) = tokens
        && (input > 0 || output > 0)
    {
        return Some((input, output, true));
    }
    match billed_total {
        Some(total) if total > 0 => Some((total, 0, true)),
        _ => None,
    }
}

#[must_use]
pub fn reconcile_io_usage(response: &ResponseObject) -> (u64, u64, bool) {
    let Some(source) = response_usage(response) else {
        return (0, 0, false);
    };
    let (input, output, cached) = match source {
        ResponseUsage::Usage(usage) => (
            usage.prompt_tokens.max(usage.input_tokens),
            usage.completion_tokens.max(usage.output_tokens),
            cached_tokens(usage),
        ),
        ResponseUsage::ResponseApi(usage) => (
            usage.input_tokens,
            usage.output_tokens,
            cached_response_api_tokens(usage),
        ),
        ResponseUsage::Mapping(usage) => (
            usage.input_tokens.max(usage.prompt_tokens),
            usage.output_tokens.max(usage.completion_tokens),
            cached_tokens(usage),
        ),
    };
    if input == 0 && output == 0 {
        return (0, 0, false);
    }
    (input.saturating_sub(cached), output, true)
}

#[must_use]
pub fn recovered_partial_usage_tokens(
    source: Option<&Usage>,
    rate_limit_type: TokenRateLimitType,
) -> (u64, u64, u64) {
    let Some(usage) = source else {
        return (0, 0, 0);
    };
    if usage.completion_tokens == 0 && usage.output_tokens == 0 {
        return (0, 0, 0);
    }
    let response = ResponseObject::Usage(usage.clone());
    let (input, output, _) = reconcile_io_usage(&response);
    (
        get_total_tokens_from_usage(response_usage(&response), rate_limit_type),
        input,
        output,
    )
}

mod tests;
