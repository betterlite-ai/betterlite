use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::tool_fees::error::{LookupFailed, RateField, ToolFeeError};
use betterlite_models::tool_fees::model_info::{Rate, SearchContextRates, ToolModelInfo};
use betterlite_models::tool_fees::ports::ToolPriceSource;
use betterlite_models::tool_fees::prices::ToolPrices;
use betterlite_models::tool_fees::scalar::{
    VectorStoreUsage, code_interpreter_fee, computer_use_fee, file_search_fee, vector_store_fee,
    web_search_rate,
};
use betterlite_models::tool_signals::conversions::{FileSearchParams, TokenCounts};
use betterlite_models::tool_signals::kwargs::{FileSearchTool, WebSearchOptions};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Number, Value, json};

use crate::helpers::{HelperError, OrHelperError as _};

type Fields = IndexMap<String, Json>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WebSearchCall {
    options: Option<Fields>,
    model_info: Option<Fields>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSearchCall {
    file_search: Option<Fields>,
    provider: Option<String>,
    model_info: Option<Fields>,
    storage_gb: Option<Number>,
    days: Option<Number>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageInput {
    storage_gb: Option<Number>,
    days: Option<Number>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorStoreCall {
    usage: UsageInput,
    provider: Option<String>,
    model_info: Option<Fields>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ComputerUseCall {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    provider: Option<String>,
    model_info: Option<Fields>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeInterpreterCall {
    sessions: Option<i64>,
    provider: Option<String>,
    model_info: Option<Fields>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    web_search: Vec<WebSearchCall>,
    file_search: Vec<FileSearchCall>,
    vector_store: Vec<VectorStoreCall>,
    computer_use: Vec<ComputerUseCall>,
    code_interpreter: Vec<CodeInterpreterCall>,
}

// The pinned model map prices the azure and openai container entries at 0.03 a session and has no other provider.
struct PinnedContainers;

impl ToolPriceSource for PinnedContainers {
    fn model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<ToolModelInfo, LookupFailed> {
        Err(LookupFailed)
    }

    fn container_session_rate(&self, provider: &str) -> Result<Rate, LookupFailed> {
        match provider {
            "azure" | "openai" => Ok(Rate::Value(Decimal::new(3, 2))),
            _ => Err(LookupFailed),
        }
    }
}

fn quantity(number: Option<&Number>) -> Result<Option<Decimal>, Box<HelperError>> {
    let Some(value) = number else {
        return Ok(None);
    };
    parse_decimal_lexeme(&value.to_string())
        .map(Some)
        .or_value_error()
}

fn rate_in(fields: &Fields, key: &str) -> Result<Rate, Box<HelperError>> {
    match fields.get(key) {
        None => Ok(Rate::Absent),
        Some(Json::Null) => Ok(Rate::Null),
        Some(Json::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map(Rate::Value)
            .or_value_error(),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err::<Rate, String>(format!("{key} must be a number or null")).or_value_error()
        }
    }
}

fn model_info(fields: &Fields) -> Result<ToolModelInfo, Box<HelperError>> {
    let search_context = match fields.get("search_context_cost_per_query") {
        Some(Json::Object(sizes)) => SearchContextRates {
            low: rate_in(sizes, "search_context_size_low")?,
            medium: rate_in(sizes, "search_context_size_medium")?,
            high: rate_in(sizes, "search_context_size_high")?,
        },
        _ => SearchContextRates::default(),
    };
    Ok(ToolModelInfo {
        litellm_provider: None,
        search_context,
        file_search_per_gb_per_day: rate_in(fields, "file_search_cost_per_gb_per_day")?,
        file_search_per_1k_calls: rate_in(fields, "file_search_cost_per_1k_calls")?,
        vector_store_per_gb_per_day: rate_in(fields, "vector_store_cost_per_gb_per_day")?,
        computer_input_per_1k: rate_in(fields, "computer_use_input_cost_per_1k_tokens")?,
        computer_output_per_1k: rate_in(fields, "computer_use_output_cost_per_1k_tokens")?,
        code_interpreter_per_session: rate_in(fields, "code_interpreter_cost_per_session")?,
    })
}

// LiteLLM returns a null web search rate or per-call file search rate as it is, and every other null meets arithmetic.
fn outcome(fee: Result<Decimal, ToolFeeError>) -> Fee {
    match fee {
        Ok(amount) => Ok(json!(amount.normalize().to_string())),
        Err(ToolFeeError::NullRate {
            field:
                RateField::SearchContextSizeLow
                | RateField::SearchContextSizeMedium
                | RateField::SearchContextSizeHigh
                | RateField::FileSearchCostPer1kCalls,
        }) => Ok(Value::Null),
        Err(error) => Err(Box::new(HelperError {
            class: "TypeError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })),
    }
}

type Fee = Result<Value, Box<HelperError>>;

impl FileSearchCall {
    fn params(&self) -> Result<FileSearchParams, Box<HelperError>> {
        Ok(FileSearchParams {
            storage_gb: quantity(self.storage_gb.as_ref())?,
            days: quantity(self.days.as_ref())?,
        })
    }
}

impl VectorStoreCall {
    fn usage(&self) -> Result<VectorStoreUsage, Box<HelperError>> {
        Ok(VectorStoreUsage {
            storage_gb: quantity(self.usage.storage_gb.as_ref())?,
            days: quantity(self.usage.days.as_ref())?,
        })
    }
}

fn optional_info(fields: Option<&Fields>) -> Result<Option<ToolModelInfo>, Box<HelperError>> {
    let Some(found) = fields else {
        return Ok(None);
    };
    model_info(found).map(Some)
}

fn web_search_call(call: &WebSearchCall) -> Fee {
    let options = call
        .options
        .clone()
        .map(|fields| WebSearchOptions { fields });
    let info = optional_info(call.model_info.as_ref())?;
    outcome(web_search_rate(options.as_ref(), info.as_ref()))
}

fn file_search_call(call: &FileSearchCall, prices: &ToolPrices) -> Fee {
    let tool = call
        .file_search
        .clone()
        .map(|fields| FileSearchTool { fields });
    let info = optional_info(call.model_info.as_ref())?;
    let provider = call.provider.as_deref();
    let params = call.params()?;
    outcome(file_search_fee(
        tool.as_ref(),
        provider,
        info.as_ref(),
        params,
        prices,
    ))
}

fn vector_store_call(call: &VectorStoreCall, prices: &ToolPrices) -> Fee {
    let info = optional_info(call.model_info.as_ref())?;
    let provider = call.provider.as_deref();
    let usage = call.usage()?;
    outcome(vector_store_fee(usage, provider, info.as_ref(), prices))
}

fn computer_use_call(call: &ComputerUseCall, prices: &ToolPrices) -> Fee {
    let info = optional_info(call.model_info.as_ref())?;
    let provider = call.provider.as_deref();
    let tokens = TokenCounts {
        input_tokens: call.input_tokens,
        output_tokens: call.output_tokens,
    };
    outcome(computer_use_fee(tokens, provider, info.as_ref(), prices))
}

fn code_interpreter_call(call: &CodeInterpreterCall) -> Fee {
    let info = optional_info(call.model_info.as_ref())?;
    let provider = call.provider.as_deref();
    outcome(code_interpreter_fee(
        call.sessions,
        provider,
        info.as_ref(),
        &PinnedContainers,
    ))
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let prices = ToolPrices::PINNED;
        let web_search = input
            .web_search
            .iter()
            .map(web_search_call)
            .collect::<Result<Vec<_>, _>>()?;
        let file_search = input
            .file_search
            .iter()
            .map(|call| file_search_call(call, &prices))
            .collect::<Result<Vec<_>, _>>()?;
        let vector_store = input
            .vector_store
            .iter()
            .map(|call| vector_store_call(call, &prices))
            .collect::<Result<Vec<_>, _>>()?;
        let computer_use = input
            .computer_use
            .iter()
            .map(|call| computer_use_call(call, &prices))
            .collect::<Result<Vec<_>, _>>()?;
        let code_interpreter = input
            .code_interpreter
            .iter()
            .map(code_interpreter_call)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "web_search": web_search,
            "file_search": file_search,
            "vector_store": vector_store,
            "computer_use": computer_use,
            "code_interpreter": code_interpreter,
        }))
    }
}

mod tests;
