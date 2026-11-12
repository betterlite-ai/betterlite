use rust_decimal::Decimal;

use crate::decisions::Json;
use crate::tool_fees::error::ToolFeeError;
use crate::tool_fees::model_info::ToolModelInfo;
use crate::tool_fees::ports::{ToolPricePolicy, ToolPriceSource};
use crate::tool_fees::prices::ToolPrices;
use crate::tool_fees::scalar::{
    VectorStoreUsage, code_interpreter_fee, computer_use_fee, file_search_fee, vector_store_fee,
    web_search_rate,
};
use crate::tool_signals::conversions::{TokenCounts, extract_file_search_params};
use crate::tool_signals::counts::count_web_search_calls;
use crate::tool_signals::kwargs::{FileSearchTool, WebSearchOptions};
use crate::tool_signals::response::{
    ToolBillingResponse, anthropic_search_usage, includes_file_search_call,
    includes_web_search_call,
};

mod tests;

#[derive(Debug, Clone, Default)]
pub struct BuiltInToolParams {
    pub web_search_options: Option<WebSearchOptions>,
    pub file_search: Option<FileSearchTool>,
    pub vector_store_usage: Option<VectorStoreUsage>,
    pub computer_use_usage: Option<TokenCounts>,
    pub code_interpreter_sessions: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct BuiltInToolRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub response: &'request ToolBillingResponse,
    // The typed prompt details do not carry this counter yet, so the caller reads it from the provider usage.
    pub google_maps_requests: Option<i64>,
    pub params: &'request BuiltInToolParams,
}

#[derive(Clone, Copy)]
pub struct BuiltInToolBilling<'deps> {
    pub source: &'deps dyn ToolPriceSource,
    pub policy: &'deps dyn ToolPricePolicy,
    pub prices: &'deps ToolPrices,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModel {
    pub info: Option<ToolModelInfo>,
    pub provider: Option<String>,
}

impl BuiltInToolBilling<'_> {
    // Maps cost joins exactly one branch: web search, else file search, else Azure assistant features.
    pub fn cost_for_built_in_tools(
        &self,
        request: &BuiltInToolRequest<'_>,
    ) -> Result<Decimal, ToolFeeError> {
        let maps = self.google_maps_cost(request);
        let branch = if includes_web_search_call(request.response) {
            self.web_search_cost(request)?
        } else if includes_file_search_call(request.response) {
            self.file_search_cost(request)?
        } else {
            self.azure_assistant_cost(request)?
        };
        maps.checked_add(branch).ok_or(ToolFeeError::Overflow)
    }

    // A truthy requested provider is kept; otherwise the entry's provider is adopted, and so is the prefix match's.
    #[must_use]
    pub fn resolve_model_info(&self, model: &str, provider: Option<&str>) -> ResolvedModel {
        if let Ok(direct) = self.source.model_info(model, provider) {
            let adopted = provider
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .or_else(|| direct.litellm_provider.clone());
            return ResolvedModel {
                info: Some(direct),
                provider: adopted,
            };
        }
        let kept = provider.map(str::to_owned);
        if !model.contains('/') {
            return ResolvedModel {
                info: None,
                provider: kept,
            };
        }
        match self.source.model_info(model, None) {
            Ok(by_prefix) => ResolvedModel {
                provider: by_prefix.litellm_provider.clone(),
                info: Some(by_prefix),
            },
            Err(_lookup) => ResolvedModel {
                info: None,
                provider: kept,
            },
        }
    }

    fn google_maps_cost(&self, request: &BuiltInToolRequest<'_>) -> Decimal {
        let Some(requests) = request.google_maps_requests else {
            return Decimal::ZERO;
        };
        let resolved = self.resolve_model_info(request.model, request.custom_llm_provider);
        let (Some(info), Some(provider)) = (&resolved.info, &resolved.provider) else {
            return Decimal::ZERO;
        };
        self.policy
            .google_maps_cost(provider, requests, info)
            .unwrap_or(Decimal::ZERO)
    }

    // A provider result wins for any non-None price, even zero; None falls back to the per-call rate.
    fn provider_web_search_price(
        &self,
        request: &BuiltInToolRequest<'_>,
        resolved: &ResolvedModel,
    ) -> Option<Decimal> {
        let repaired = anthropic_search_usage(request.response);
        let (Some(info), Some(usage), Some(provider)) =
            (&resolved.info, &repaired, &resolved.provider)
        else {
            return None;
        };
        self.policy.web_search_cost(provider, usage, info)
    }

    fn web_search_cost(&self, request: &BuiltInToolRequest<'_>) -> Result<Decimal, ToolFeeError> {
        let resolved = self.resolve_model_info(request.model, request.custom_llm_provider);
        if let Some(price) = self.provider_web_search_price(request, &resolved) {
            return Ok(price);
        }
        let options = request.params.web_search_options.as_ref();
        let per_call = web_search_rate(options, resolved.info.as_ref())?;
        let calls = count_web_search_calls(request.response)?;
        per_call
            .checked_mul(Decimal::from(calls))
            .ok_or(ToolFeeError::Overflow)
    }

    // An empty file_search object is falsey here and costs nothing, unlike the scalar function.
    fn file_search_cost(&self, request: &BuiltInToolRequest<'_>) -> Result<Decimal, ToolFeeError> {
        let info = self
            .source
            .model_info(request.model, request.custom_llm_provider);
        let tool = request
            .params
            .file_search
            .as_ref()
            .filter(|tool| !tool.fields.is_empty());
        let usage = tool.map(|found| Json::Object(found.fields.clone()));
        let params = extract_file_search_params(usage.as_ref())?;
        let provider = request.custom_llm_provider;
        file_search_fee(tool, provider, info.ok().as_ref(), params, self.prices)
    }

    fn azure_assistant_cost(
        &self,
        request: &BuiltInToolRequest<'_>,
    ) -> Result<Decimal, ToolFeeError> {
        let provider = request.custom_llm_provider;
        if provider != Some("azure") {
            return Ok(Decimal::ZERO);
        }
        let info = self.source.model_info(request.model, provider).ok();
        let billable = request
            .params
            .vector_store_usage
            .filter(|usage| usage.storage_gb.is_some() || usage.days.is_some());
        let vector_fee = match billable {
            Some(usage) => vector_store_fee(usage, provider, info.as_ref(), self.prices)?,
            None => Decimal::ZERO,
        };
        let computer_fee = computer_use_fee(
            request.params.computer_use_usage.unwrap_or_default(),
            provider,
            info.as_ref(),
            self.prices,
        )?;
        let interpreter_fee = code_interpreter_fee(
            request.params.code_interpreter_sessions,
            provider,
            info.as_ref(),
            self.source,
        )?;
        [vector_fee, computer_fee, interpreter_fee]
            .into_iter()
            .try_fold(Decimal::ZERO, Decimal::checked_add)
            .ok_or(ToolFeeError::Overflow)
    }
}
