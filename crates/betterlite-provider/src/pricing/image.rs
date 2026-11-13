pub mod error;
mod tests;

use betterlite_bedrock::image_models::params::Params;
use betterlite_bedrock::image_pricing::{ImageCostRequest, image_cost};
use betterlite_core::cost::image_dispatch::options::OptionalParams;
use betterlite_core::cost::image_dispatch::policy::{
    ImagePricePolicy, ImageResponseView, ImageRoute, PricedImage,
};
use betterlite_models::image_cost::ImageBilling;
use betterlite_models::image_cost::azure::{
    AzureImageCostRequest, AzureImageResponse, cost_calculator as azure_ai_cost,
};
use betterlite_models::image_cost::default_calculator::{
    DefaultImageCostRequest, default_image_cost_calculator,
};
use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::openai::{
    OpenAiImageCostRequest, cost_calculator as openai_cost,
};
use betterlite_models::price_lookup::PriceLookup;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;
use serde_json::value::to_raw_value;

use error::ImagePolicyError;

// The calculators whose arithmetic already lives in the models and bedrock crates; the other providers' owners are not installed yet.
pub struct ProviderImagePricing<'env> {
    pub lookup: &'env dyn PriceLookup,
    pub database: &'env TimeZoneDatabase,
    pub moment: DateTime<Utc>,
}

const fn parsed<'image>(
    image: &PricedImage<'image>,
) -> Result<&'image ImageBilling, ImagePolicyError> {
    match image.response {
        ImageResponseView::Image(billing) => Ok(billing),
        ImageResponseView::Other => Err(ImagePolicyError::Image(ImageCostError::NotImageResponse)),
    }
}

// Bedrock reads the request options through its own typed parameter map, so they travel as JSON text.
fn params_from(options: Option<&OptionalParams>) -> Result<Option<Params>, serde_json::Error> {
    let Some(map) = options else {
        return Ok(None);
    };
    let text = serde_json::to_string(map)?;
    serde_json::from_str(&text).map(Some)
}

impl ProviderImagePricing<'_> {
    fn gpt_image(
        &self,
        image: &PricedImage<'_>,
        provider: &str,
    ) -> Result<Decimal, ImagePolicyError> {
        let billing = parsed(image)?;
        let request = OpenAiImageCostRequest {
            model: image.model,
            custom_llm_provider: Some(provider),
            model_info: image.prices,
            image_count: billing.image_count,
            usage: billing.usage.as_ref(),
            moment: self.moment,
        };
        Ok(openai_cost(&request, self.lookup, self.database)?)
    }

    fn default_price(&self, route: &ImageRoute<'_>) -> Result<Decimal, ImagePolicyError> {
        let ImageRoute::Default {
            image,
            provider,
            quality,
            image_count,
            size,
            ..
        } = route
        else {
            return Err(ImagePolicyError::UnavailableAdapter(route.adapter()));
        };
        let request = DefaultImageCostRequest {
            model: image.model,
            custom_llm_provider: *provider,
            quality: Some(quality),
            n_images: Some(*image_count),
            size: Some(size),
            model_info: image.prices,
        };
        Ok(default_image_cost_calculator(&request, self.lookup)?)
    }

    fn azure_ai(
        &self,
        image: &PricedImage<'_>,
        size: &str,
        image_count: i64,
        optional_params: Option<&OptionalParams>,
    ) -> Result<Decimal, ImagePolicyError> {
        let raw = |key: &str| {
            optional_params
                .and_then(|options| options.get(key))
                .map(to_raw_value)
                .transpose()
                .map_err(|source| ImagePolicyError::OptionalParams { source })
        };
        let (width, height) = (raw("width")?, raw("height")?);
        let response = match image.response {
            ImageResponseView::Image(billing) => AzureImageResponse::Image(billing),
            ImageResponseView::Other => AzureImageResponse::Other,
        };
        let request = AzureImageCostRequest {
            model: image.model,
            model_info: image.prices,
            size: Some(size),
            n_images: Some(image_count),
            width: width.as_deref(),
            height: height.as_deref(),
            response,
            moment: self.moment,
        };
        Ok(azure_ai_cost(&request, self.lookup, self.database)?)
    }

    fn bedrock(
        &self,
        model: &str,
        size: &str,
        response: &ImageBilling,
        optional_params: Option<&OptionalParams>,
    ) -> Result<Decimal, ImagePolicyError> {
        let params = params_from(optional_params)
            .map_err(|source| ImagePolicyError::OptionalParams { source })?;
        let request = ImageCostRequest {
            model,
            image_count: response.image_count,
            size: Some(size),
            optional_params: params.as_ref(),
        };
        Ok(image_cost(&request, self.lookup)?)
    }
}

impl ImagePricePolicy for ProviderImagePricing<'_> {
    type Error = ImagePolicyError;

    fn image_cost(&self, route: &ImageRoute<'_>) -> Result<Decimal, ImagePolicyError> {
        match route {
            ImageRoute::GptImage { image, provider } => self.gpt_image(image, provider),
            ImageRoute::AzureAi {
                image,
                size,
                image_count,
                optional_params,
            } => self.azure_ai(image, size, *image_count, *optional_params),
            ImageRoute::Bedrock {
                model,
                size,
                response,
                optional_params,
            } => self.bedrock(model, size, response, *optional_params),
            ImageRoute::Default { .. } => self.default_price(route),
            ImageRoute::VertexAi(_)
            | ImageRoute::Recraft(_)
            | ImageRoute::Aiml(_)
            | ImageRoute::Cometapi(_)
            | ImageRoute::Gemini(_)
            | ImageRoute::GeminiEdit(_)
            | ImageRoute::FalAi { .. }
            | ImageRoute::Runwayml(_) => Err(ImagePolicyError::UnavailableAdapter(route.adapter())),
        }
    }
}
