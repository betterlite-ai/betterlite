use betterlite_openai::api::image_contracts::records::ImageInputTokens;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_vertex::studio_image_generation::cost::{ImageCostResponse, generation_cost};
use betterlite_vertex::studio_image_generation::response::ImageGenerationResponse;
use betterlite_vertex::studio_image_generation::usage::ImageGenerationUsage;
use betterlite_vertex::vertex_gemini_images::response::GeneratedImage;
use serde_json::{Value, json};

use super::ops::{CostCase, CostUsage};
use super::pricing::CatalogPricing;

pub(super) fn cost_value(case: &CostCase) -> Result<Value, Box<HelperError>> {
    let response = ImageGenerationResponse {
        data: image_records(case.images),
        usage: case.usage.as_ref().map(cost_usage),
    };
    let pricing = CatalogPricing {
        catalog: &case.catalog,
    };
    let cost = generation_cost(
        &pricing,
        &case.model,
        None,
        ImageCostResponse::Image(&response),
    )
    .or_value_error()?;
    Ok(json!({"cost": cost.normalize().to_string()}))
}

fn image_records(count: usize) -> Vec<GeneratedImage> {
    let image = GeneratedImage {
        b64_json: Some("AAA".to_owned()),
        url: None,
        revised_prompt: None,
        provider_specific_fields: None,
    };
    vec![image; count]
}

// the usage the response would carry: the itemized input details and the images taking the whole output
fn cost_usage(usage: &CostUsage) -> ImageGenerationUsage {
    let input_details = ImageInputTokens {
        image_tokens: usage.image_tokens,
        text_tokens: usage.text_tokens,
    };
    let output_details = ImageInputTokens {
        image_tokens: usage.output_tokens,
        text_tokens: 0,
    };
    ImageGenerationUsage {
        input_tokens: usage.input_tokens,
        input_tokens_details: input_details.clone(),
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
        prompt_tokens: usage.input_tokens,
        prompt_tokens_details: input_details,
        completion_tokens: usage.output_tokens,
        completion_tokens_details: output_details.clone(),
        output_tokens_details: output_details,
        web_search_requests: None,
    }
}
