use std::collections::BTreeMap;

use betterlite_models::cost_usage::tool_projection::ToolUsage;
use betterlite_models::cost_usage::{CostUsage, PromptDetails};
use betterlite_models::tool_fees::model_info::ToolModelInfo;
use betterlite_models::tool_fees::ports::ToolPricePolicy;
use rust_decimal::Decimal;

// calculate_image_response_web_search_cost: a missing usage or a zero count is free, otherwise the count rides a chat-shaped usage through the web-search dispatch and a missing or zero answer is zero.
pub fn image_grounding_cost(
    policy: &dyn ToolPricePolicy,
    provider: &str,
    web_search_requests: Option<i64>,
    model_info: &ToolModelInfo,
) -> Decimal {
    let Some(requests) = web_search_requests.filter(|count| *count != 0) else {
        return Decimal::ZERO;
    };
    let usage = ToolUsage {
        cost: CostUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            prompt_details: Some(PromptDetails {
                text_tokens: None,
                audio_tokens: None,
                image_tokens: None,
                video_tokens: None,
                web_search_requests: Some(requests),
            }),
            completion_details: None,
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        },
        server_tool_use: None,
        server_side_details: None,
    };
    policy
        .web_search_cost(provider, &usage, model_info)
        .unwrap_or(Decimal::ZERO)
}
