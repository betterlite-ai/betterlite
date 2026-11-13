use std::error::Error;

use betterlite_models::image_cost::ImageBilling;
use betterlite_models::price_lookup::PriceRecord;
use rust_decimal::Decimal;

use super::options::OptionalParams;

// The response as the dispatcher received it: a parsed ImageResponse, or anything else.
#[derive(Debug, Clone, Copy)]
pub enum ImageResponseView<'response> {
    Image(&'response ImageBilling),
    Other,
}

// The arguments of a provider calculator that takes only the model, the response and the deployment prices.
#[derive(Debug, Clone, Copy)]
pub struct PricedImage<'request> {
    pub model: &'request str,
    pub response: ImageResponseView<'request>,
    pub prices: Option<&'request PriceRecord>,
}

// One variant per calculator the dispatcher can pick, holding exactly the arguments LiteLLM passes it.
#[derive(Debug)]
pub enum ImageRoute<'request> {
    // Only an ImageResponse is priced by Vertex; any other response costs zero without reaching the policy.
    VertexAi(PricedImage<'request>),
    // Bedrock never receives the deployment prices.
    Bedrock {
        model: &'request str,
        size: &'request str,
        response: &'request ImageBilling,
        optional_params: Option<&'request OptionalParams>,
    },
    Recraft(PricedImage<'request>),
    Aiml(PricedImage<'request>),
    Cometapi(PricedImage<'request>),
    Gemini(PricedImage<'request>),
    GeminiEdit(PricedImage<'request>),
    Runwayml(PricedImage<'request>),
    AzureAi {
        image: PricedImage<'request>,
        size: &'request str,
        image_count: i64,
        optional_params: Option<&'request OptionalParams>,
    },
    FalAi {
        image: PricedImage<'request>,
        optional_params: Option<&'request OptionalParams>,
    },
    GptImage {
        image: PricedImage<'request>,
        provider: &'request str,
    },
    Default {
        image: PricedImage<'request>,
        provider: Option<&'request str>,
        quality: &'request str,
        image_count: i64,
        size: &'request str,
        optional_params: Option<&'request OptionalParams>,
    },
}

// The provider layer implements the arithmetic of every route; the dispatcher only chooses which one runs.
pub trait ImagePricePolicy {
    type Error: Error + 'static;

    fn image_cost(&self, route: &ImageRoute<'_>) -> Result<Decimal, Self::Error>;
}
