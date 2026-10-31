use betterlite_models::off_peak::schema::OffPeakBlockEntry;
use betterlite_models::tiered_pricing::Tier;
use betterlite_models::token_rates::{
    self, ModelInfo, ModelPricing, RequestContext, TokenRateError, Usage,
};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;
use thiserror::Error;

pub(crate) struct Op;

const fn default_moment() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

#[derive(Deserialize)]
pub(crate) struct BaseInput {
    model_info: ModelInfo,
    usage: Usage,
    service_tier: Option<String>,
    provider: Option<String>,
    #[serde(default)]
    tiered_pricing: Vec<Tier>,
    #[serde(default)]
    off_peak_pricing: Option<OffPeakBlockEntry>,
    #[serde(default = "default_moment")]
    moment: DateTime<Utc>,
}

#[derive(Deserialize)]
#[serde(tag = "operation")]
pub(crate) enum Input {
    #[serde(rename = "base")]
    Base(Box<BaseInput>),
    #[serde(rename = "batch")]
    Batch {
        model_info: ModelInfo,
        usage: Usage,
        provider: Option<String>,
    },
    #[serde(rename = "component")]
    Component {
        model_info: ModelInfo,
        cost_key: String,
        usage: Option<Decimal>,
    },
    #[serde(rename = "deployment")]
    Deployment { model_info: Option<ModelInfo> },
}

#[derive(Debug, Error)]
enum RunError {
    #[error(transparent)]
    Token(#[from] TokenRateError),
    #[error(transparent)]
    Serialize(#[from] serde_json::Error),
}

impl From<RunError> for Box<crate::helpers::HelperError> {
    fn from(error: RunError) -> Self {
        let class = match &error {
            RunError::Token(_) => "TokenRateError",
            RunError::Serialize(_) => "Serialize",
        };
        Self::new(crate::helpers::HelperError {
            class: class.to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: error.to_string(),
        })
    }
}

fn run_base(fields: &BaseInput) -> Result<serde_json::Value, RunError> {
    let database = TimeZoneDatabase::from_env();
    let pricing = ModelPricing {
        model: &fields.model_info,
        tiered_pricing: &fields.tiered_pricing,
        off_peak_pricing: fields
            .off_peak_pricing
            .as_ref()
            .and_then(OffPeakBlockEntry::as_block),
    };
    let context = RequestContext {
        service_tier: fields.service_tier.as_deref(),
        provider: fields.provider.as_deref(),
        moment: fields.moment,
        database: &database,
    };
    let rates = token_rates::base_cost::get_token_base_cost(&pricing, fields.usage, &context)?;
    Ok(serde_json::json!({
        "input_rate": rates.input_rate.normalize().to_string(),
        "output_rate": rates.output_rate.normalize().to_string(),
        "cache_read_rate": rates.cache_read_rate.normalize().to_string(),
        "cache_creation_rate": rates.cache_creation_rate.normalize().to_string(),
        "cache_creation_above_1hr_rate": rates.cache_creation_above_1hr_rate.normalize().to_string(),
        "reasoning_rate": rates.reasoning_rate.normalize().to_string(),
    }))
}

fn run_batch(
    model_info: &ModelInfo,
    usage: Usage,
    provider: Option<&str>,
) -> Result<serde_json::Value, RunError> {
    let rates = token_rates::batch::get_batch_cost_rates(model_info, usage, provider)?;
    Ok(serde_json::to_value(rates)?)
}

fn run_component(
    model_info: &ModelInfo,
    cost_key: &str,
    usage: Option<Decimal>,
) -> Result<serde_json::Value, RunError> {
    let component = token_rates::component::calculate_cost_component(model_info, cost_key, usage)?;
    Ok(serde_json::to_value(component.normalize().to_string())?)
}

fn run_deployment(model_info: Option<&ModelInfo>) -> Result<serde_json::Value, RunError> {
    let pricing = token_rates::deployment::deployment_pricing(model_info)?;
    Ok(serde_json::to_value(pricing)?)
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::Base(fields) => run_base(fields),
            Input::Batch {
                model_info,
                usage,
                provider,
            } => run_batch(model_info, *usage, provider.as_deref()),
            Input::Component {
                model_info,
                cost_key,
                usage,
            } => run_component(model_info, cost_key, *usage),
            Input::Deployment { model_info } => run_deployment(model_info.as_ref()),
        };
        Ok(result?)
    }
}
