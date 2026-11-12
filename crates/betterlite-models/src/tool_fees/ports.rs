use rust_decimal::Decimal;

use crate::cost_usage::tool_projection::ToolUsage;
use crate::tool_fees::error::LookupFailed;
use crate::tool_fees::model_info::{Rate, ToolModelInfo};

// The catalog side: a failed lookup is an Err here and becomes absent info where the fee code calls it.
pub trait ToolPriceSource {
    fn model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<ToolModelInfo, LookupFailed>;

    // The raw code_interpreter_cost_per_session of the entry that "{provider}/container" resolves to.
    fn container_session_rate(&self, provider: &str) -> Result<Rate, LookupFailed>;
}

// The provider layer supplies these; None means the provider has no formula, which differs from an explicit zero.
pub trait ToolPricePolicy {
    fn web_search_cost(
        &self,
        provider: &str,
        usage: &ToolUsage,
        model_info: &ToolModelInfo,
    ) -> Option<Decimal>;

    fn google_maps_cost(
        &self,
        provider: &str,
        requests: i64,
        model_info: &ToolModelInfo,
    ) -> Option<Decimal>;
}
