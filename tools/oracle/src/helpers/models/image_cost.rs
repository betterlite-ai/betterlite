mod input;

use betterlite_models::image_cost::azure::{self, AzureImageCostRequest, AzureImageResponse};
use betterlite_models::image_cost::default_calculator::{
    DefaultImageCostRequest, default_image_cost_calculator,
};
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::openai::{self, OpenAiImageCostRequest};
use betterlite_models::image_cost::usage::{ImageUsageCostRequest, image_usage_cost};
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use jiff::tz::TimeZoneDatabase;
use serde_json::json;

use input::Input;

use crate::helpers::HelperError;

pub(crate) struct Op;

// No case for this task replaces litellm.model_cost; every fixture carries its own explicit price table.
struct NullLookup;

impl PriceLookup for NullLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

fn image_cost_error(error: &ImageCostError) -> Box<HelperError> {
    let class = match error {
        ImageCostError::UnknownModel { .. } => "UnknownModel",
        ImageCostError::NotImageResponse => "NotImageResponse",
        ImageCostError::NoPrice { .. } => "NoPrice",
        ImageCostError::WrongFieldType { .. } => "WrongFieldType",
        ImageCostError::MalformedSize { .. } => "MalformedSize",
        ImageCostError::DimensionOverflow => "DimensionOverflow",
        ImageCostError::CounterOverflow => "CounterOverflow",
        ImageCostError::Overflow => "Overflow",
        ImageCostError::Rate(_) => "RateError",
        ImageCostError::Tier(_) => "TierError",
        ImageCostError::TokenCost(_) => "TokenCostError",
    };
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn run_default(fields: &input::DefaultInput) -> crate::helpers::HelperResult {
    let cost = default_image_cost_calculator(
        &DefaultImageCostRequest {
            model: &fields.model,
            custom_llm_provider: fields.custom_llm_provider.as_deref(),
            quality: fields.quality.as_deref(),
            n_images: fields.n_images,
            size: fields.size.as_deref(),
            model_info: fields.model_info.as_ref(),
        },
        &NullLookup,
    )
    .map_err(|error| image_cost_error(&error))?;
    Ok(json!({ "cost": cost.normalize().to_string() }))
}

fn run_openai(fields: &input::OpenaiInput) -> crate::helpers::HelperResult {
    let view = fields
        .usage
        .clone()
        .map(input::UsageViewInput::into_view)
        .transpose()?;
    let database = TimeZoneDatabase::from_env();
    let cost = openai::cost_calculator(
        &OpenAiImageCostRequest {
            model: &fields.model,
            custom_llm_provider: fields.custom_llm_provider.as_deref(),
            model_info: fields.model_info.as_ref(),
            image_count: fields.image_count,
            usage: view.as_ref(),
            moment: fields.moment,
        },
        &NullLookup,
        &database,
    )
    .map_err(|error| image_cost_error(&error))?;
    Ok(json!({ "cost": cost.normalize().to_string() }))
}

fn run_azure(fields: &input::AzureInput) -> crate::helpers::HelperResult {
    let width = input::raw_value(fields.width.as_ref())?;
    let height = input::raw_value(fields.height.as_ref())?;
    let billing = fields.billing.clone().into_billing()?;
    let database = TimeZoneDatabase::from_env();
    let cost = azure::cost_calculator(
        &AzureImageCostRequest {
            model: &fields.model,
            model_info: fields.model_info.as_ref(),
            size: fields.size.as_deref(),
            n_images: fields.n_images,
            width: width.as_deref(),
            height: height.as_deref(),
            response: AzureImageResponse::Image(&billing),
            moment: fields.moment,
        },
        &NullLookup,
        &database,
    )
    .map_err(|error| image_cost_error(&error))?;
    Ok(json!({ "cost": cost.normalize().to_string() }))
}

fn run_usage(fields: &input::UsageOpInput) -> crate::helpers::HelperResult {
    let details = fields.usage.to_details()?;
    let database = TimeZoneDatabase::from_env();
    let cost = image_usage_cost(
        &ImageUsageCostRequest {
            model: &fields.model,
            custom_llm_provider: fields.custom_llm_provider.as_deref(),
            usage: &details,
            model_info: fields.model_info.as_ref(),
            moment: fields.moment,
        },
        &NullLookup,
        &database,
    )
    .map_err(|error| image_cost_error(&error))?;
    Ok(json!({ "cost": cost.map(|value| value.normalize().to_string()) }))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::Default(fields) => run_default(fields),
            Input::Openai(fields) => run_openai(fields),
            Input::Azure(fields) => run_azure(fields),
            Input::Usage(fields) => run_usage(fields),
        }
    }
}
