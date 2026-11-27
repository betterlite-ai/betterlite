use betterlite_bedrock::image_edits::plan::{EditEnvironment, EditImage, EditPlan};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::bedrock_image_dispatch::call::{BedrockImageOperation, EditOperation};
use betterlite_provider::bedrock_image_dispatch::test_support::{Catalog, NOW_SECONDS, instant};
use serde_json::Value;

use super::super::{Input, Probe};
use super::{exchange_row, header_pairs};
use crate::openai_media_dispatch::failure;

// One edit probe: the call the caller resolved, exchanged through the production dispatcher
pub(super) async fn edit_row(input: &Input, probe: &Probe) -> Result<Value, Box<HelperError>> {
    let images: Vec<EditImage> = probe.images.iter().cloned().map(EditImage::Text).collect();
    let catalog = Catalog::empty().map_err(|error| failure("Catalog", error))?;
    let context = catalog.context();
    let extra = probe.extra_headers.as_ref().map(header_pairs);
    let operation = EditOperation {
        model: Some(probe.model.as_str()),
        prompt: probe.prompt.as_deref(),
        images: &images,
        optional_params: probe.optional_params.clone(),
        extra_headers: extra.as_deref(),
        api_base: None,
        api_key: None,
        timeout: None,
        aimage_edit: false,
        environment: EditEnvironment::default(),
        now: instant(NOW_SECONDS).map_err(|error| failure("Clock", error))?,
        plan: EditPlan {
            model_cost: &input.model_cost,
            resolution: &context,
        },
    };
    exchange_row(BedrockImageOperation::Edit(operation), probe).await
}
