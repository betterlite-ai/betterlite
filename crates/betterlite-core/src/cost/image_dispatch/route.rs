use std::collections::BTreeMap;
use std::error::Error;

use betterlite_models::image_cost::ImageBilling;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_rates::ModelInfo;
use betterlite_models::token_rates::deployment::deployment_pricing;
use rust_decimal::Decimal;

use super::error::ImageDispatchError;
use super::options::{OptionalParams, SizePattern, requested_image_param};
use super::policy::{ImagePricePolicy, ImageResponseView, ImageRoute, PricedImage};

const DEFAULT_SIZE: &str = "1024-x-1024";
const DEFAULT_QUALITY: &str = "standard";

// Everything route_image_generation_cost_calculator reads; the call type stays a string since passthrough has no CallType.
pub struct ImageCall<'call> {
    pub model: &'call str,
    pub custom_llm_provider: Option<&'call str>,
    pub quality: Option<&'call str>,
    pub image_count: Option<i64>,
    pub size: Option<&'call str>,
    pub optional_params: Option<&'call OptionalParams>,
    pub call_type: Option<&'call str>,
    pub model_info: Option<&'call ModelInfo>,
    pub response: ImageResponseView<'call>,
}

struct Resolved<'call> {
    size: &'call str,
    quality: &'call str,
    image_count: i64,
}

// The deployment's valid prices as the catalog-shaped record the provider calculators read.
fn deployment_prices<E>(
    model_info: Option<&ModelInfo>,
) -> Result<Option<PriceRecord>, ImageDispatchError<E>>
where
    E: Error + 'static,
{
    let Some(pricing) =
        deployment_pricing(model_info).map_err(|source| ImageDispatchError::Rates { source })?
    else {
        return Ok(None);
    };
    let text =
        serde_json::to_string(&pricing).map_err(|source| ImageDispatchError::Prices { source })?;
    let fields: BTreeMap<String, String> =
        serde_json::from_str(&text).map_err(|source| ImageDispatchError::Prices { source })?;
    let mut record = PriceRecord::new();
    for (key, price) in fields {
        let number = price
            .parse()
            .map_err(|source| ImageDispatchError::Prices { source })?;
        record.insert(key, Scalar::Number(number));
    }
    Ok(Some(record))
}

// Explicit arguments beat the response, which beats the request options, which beat the defaults; an empty text is skipped.
fn resolve<'call>(
    call: &ImageCall<'call>,
    billing: Option<&'call ImageBilling>,
    size_pattern: &SizePattern,
) -> Resolved<'call> {
    let optional_params = call.optional_params;
    let sizes = [
        call.size,
        billing.and_then(|image| image.size.as_deref()),
        size_pattern.requested_size(optional_params),
    ];
    let qualities = [
        call.quality,
        billing.and_then(|image| image.quality.as_deref()),
        requested_image_param(optional_params, "quality"),
    ];
    Resolved {
        size: sizes
            .into_iter()
            .flatten()
            .find(|text| !text.is_empty())
            .unwrap_or(DEFAULT_SIZE),
        quality: qualities
            .into_iter()
            .flatten()
            .find(|text| !text.is_empty())
            .unwrap_or(DEFAULT_QUALITY),
        image_count: call
            .image_count
            .unwrap_or_else(|| billing.map_or(0, |image| image.image_count)),
    }
}

// The providers whose calculator takes only the model, the response and the deployment prices.
fn priced_route<'call>(
    call: &ImageCall<'call>,
    image: PricedImage<'call>,
) -> Option<ImageRoute<'call>> {
    match (call.custom_llm_provider, call.call_type) {
        (Some(name @ ("openai" | "azure")), _)
            if call.model.to_lowercase().contains("gpt-image") =>
        {
            Some(ImageRoute::GptImage {
                image,
                provider: name,
            })
        }
        (Some("recraft"), _) => Some(ImageRoute::Recraft(image)),
        (Some("aiml"), _) => Some(ImageRoute::Aiml(image)),
        (Some("cometapi"), _) => Some(ImageRoute::Cometapi(image)),
        (Some("gemini"), Some("image_edit" | "aimage_edit")) => Some(ImageRoute::GeminiEdit(image)),
        (Some("gemini"), _) => Some(ImageRoute::Gemini(image)),
        (Some("runwayml"), _) => Some(ImageRoute::Runwayml(image)),
        _ => None,
    }
}

// The calculator LiteLLM picks for the provider, or None where a non-image Vertex response prices at zero.
fn select_route<'call, E>(
    call: &ImageCall<'call>,
    billing: Option<&'call ImageBilling>,
    resolved: &Resolved<'call>,
    image: PricedImage<'call>,
) -> Result<Option<ImageRoute<'call>>, ImageDispatchError<E>>
where
    E: Error + 'static,
{
    let Resolved {
        size,
        quality,
        image_count,
    } = *resolved;
    let optional_params = call.optional_params;
    let provider = call.custom_llm_provider;
    let route = match provider {
        Some("vertex_ai") => match billing {
            Some(_) => ImageRoute::VertexAi(image),
            None => return Ok(None),
        },
        Some("bedrock") => ImageRoute::Bedrock {
            model: call.model,
            size,
            response: billing.ok_or(ImageDispatchError::NotImageResponse)?,
            optional_params,
        },
        Some("azure_ai") => ImageRoute::AzureAi {
            image,
            size,
            image_count,
            optional_params,
        },
        Some("fal_ai") => ImageRoute::FalAi {
            image,
            optional_params,
        },
        _ => priced_route(call, image).unwrap_or(ImageRoute::Default {
            image,
            provider,
            quality,
            image_count,
            size,
            optional_params,
        }),
    };
    Ok(Some(route))
}

// Chooses the calculator for the provider and runs it through the installed policy.
pub fn route_image_cost<P>(
    policy: &P,
    size_pattern: &SizePattern,
    call: &ImageCall<'_>,
) -> Result<Decimal, ImageDispatchError<P::Error>>
where
    P: ImagePricePolicy,
{
    let billing = match call.response {
        ImageResponseView::Image(billing) => Some(billing),
        ImageResponseView::Other => None,
    };
    let resolved = resolve(call, billing, size_pattern);
    let prices = deployment_prices(call.model_info)?;
    let image = PricedImage {
        model: call.model,
        response: call.response,
        prices: prices.as_ref(),
    };
    select_route(call, billing, &resolved, image)?.map_or_else(
        || Ok(Decimal::ZERO),
        |route| {
            policy
                .image_cost(&route)
                .map_err(ImageDispatchError::Policy)
        },
    )
}
