use betterlite_models::off_peak::schema::OffPeakBlockEntry;
use betterlite_models::tiered_pricing::Tier;
use betterlite_models::token_rates::{self, ModelInfo, ModelPricing, RequestContext, Usage};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde::Deserialize;

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

fn helper_error(error: impl std::fmt::Display) -> Box<crate::helpers::HelperError> {
    let wrapped = crate::helpers::HelperError {
        class: "TokenRateError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    };
    Box::new(wrapped)
}

fn run_base(fields: &BaseInput) -> crate::helpers::HelperResult {
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
    let rates = token_rates::base_cost::get_token_base_cost(&pricing, fields.usage, &context)
        .map_err(helper_error)?;
    Ok(serde_json::json!({
        "input_rate": rates.input_rate.normalize().to_string(),
        "output_rate": rates.output_rate.normalize().to_string(),
        "cache_read_rate": rates.cache_read_rate.normalize().to_string(),
        "cache_creation_rate": rates.cache_creation_rate.normalize().to_string(),
        "cache_creation_above_1hr_rate": rates.cache_creation_above_1hr_rate.normalize().to_string(),
        "reasoning_rate": rates.reasoning_rate.normalize().to_string(),
    }))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Base(fields) => run_base(fields),
            Input::Batch {
                model_info,
                usage,
                provider,
            } => serde_json::to_value(
                token_rates::batch::get_batch_cost_rates(model_info, *usage, provider.as_deref())
                    .map_err(helper_error)?,
            )
            .map_err(helper_error),
            Input::Component {
                model_info,
                cost_key,
                usage,
            } => serde_json::to_value(
                token_rates::deployment::calculate_cost_component(model_info, cost_key, *usage)
                    .map_err(helper_error)?
                    .normalize()
                    .to_string(),
            )
            .map_err(helper_error),
            Input::Deployment { model_info } => serde_json::to_value(
                token_rates::deployment::deployment_pricing(model_info.as_ref())
                    .map_err(helper_error)?,
            )
            .map_err(helper_error),
        }
    }
}
