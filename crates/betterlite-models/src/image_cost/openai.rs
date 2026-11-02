use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::ImageUsageView;
use super::error::ImageCostError;
use super::pricing::{flat_image_cost, resolve_image_model_info};
use super::usage::{
    ChatUsageCostRequest, ImageUsageCostRequest, ImageUsageDetails, chat_usage_cost,
    image_usage_cost,
};
use crate::cost_usage::CostUsage;
use crate::price_lookup::{PriceLookup, PriceRecord};

pub struct OpenAiImageCostRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub model_info: Option<&'request PriceRecord>,
    pub image_count: i64,
    pub usage: Option<&'request ImageUsageView>,
    pub moment: DateTime<Utc>,
}

// A falsey provider defaults to openai, but the price table itself stays absent when the caller supplied no model_info at all: no usage then costs zero rather than performing a public flat-price lookup.
pub fn cost_calculator(
    request: &OpenAiImageCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<Decimal, ImageCostError> {
    let provider = Some(
        request
            .custom_llm_provider
            .filter(|value| !value.is_empty())
            .unwrap_or("openai"),
    );
    let price_table = request
        .model_info
        .map(|model_info| {
            resolve_image_model_info(request.model, provider, Some(model_info), lookup)
        })
        .transpose()?;

    let Some(usage) = request.usage else {
        return flat_image_cost(price_table.as_ref(), request.image_count);
    };

    priced_usage(
        &PricingContext {
            request,
            provider,
            price_table: price_table.as_ref(),
        },
        usage,
        lookup,
        database,
    )
}

struct PricingContext<'request> {
    request: &'request OpenAiImageCostRequest<'request>,
    provider: Option<&'request str>,
    price_table: Option<&'request PriceRecord>,
}

// Chat Usage with a completion detail breakdown goes directly through generic pricing; otherwise try image usage when input_tokens is present, then any chat Usage as text, then flat cost.
fn priced_usage(
    context: &PricingContext<'_>,
    usage: &ImageUsageView,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<Decimal, ImageCostError> {
    match usage {
        ImageUsageView::Chat(chat_usage) if chat_usage.completion_details.is_some() => {
            priced_chat(context, chat_usage, lookup, database)
        }
        ImageUsageView::Image(details) if details.input_tokens.is_some() => {
            priced_image(context, details, lookup, database)
        }
        ImageUsageView::Chat(chat_usage) => priced_chat(context, chat_usage, lookup, database),
        ImageUsageView::Image(_) => {
            flat_image_cost(context.price_table, context.request.image_count)
        }
    }
}

fn priced_chat(
    context: &PricingContext<'_>,
    chat_usage: &CostUsage,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<Decimal, ImageCostError> {
    let request = context.request;
    chat_usage_cost(
        &ChatUsageCostRequest {
            model: request.model,
            custom_llm_provider: context.provider,
            usage: chat_usage,
            model_info: context.price_table,
            moment: request.moment,
        },
        lookup,
        database,
    )
}

fn priced_image(
    context: &PricingContext<'_>,
    details: &ImageUsageDetails,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<Decimal, ImageCostError> {
    let request = context.request;
    let token_based = image_usage_cost(
        &ImageUsageCostRequest {
            model: request.model,
            custom_llm_provider: context.provider,
            usage: details,
            model_info: context.price_table,
            moment: request.moment,
        },
        lookup,
        database,
    )?;
    token_based.map_or_else(
        || flat_image_cost(context.price_table, request.image_count),
        Ok,
    )
}

mod tests;
