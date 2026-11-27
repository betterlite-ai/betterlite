use betterlite_bedrock::image_call::plan::ImageEnvironment;
use betterlite_bedrock::image_models::params::Params;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::bedrock_image_dispatch::call::{BedrockImageOperation, GenerateOperation};
use betterlite_provider::bedrock_image_dispatch::test_support::{NOW_SECONDS, instant};
use serde_json::Value;

use super::super::Probe;
use super::{exchange_row, header_pairs};
use crate::openai_media_dispatch::failure;

// One generation probe: the call the caller resolved, exchanged through the production dispatcher
pub(super) async fn generation_row(probe: &Probe) -> Result<Value, Box<HelperError>> {
    let prompt = probe
        .prompt
        .as_deref()
        .ok_or_else(|| failure("Prompt", "a generation probe carries a prompt"))?;
    let extra = header_pairs(probe.extra_headers.as_ref());
    let operation = GenerateOperation {
        model: Some(probe.model.as_str()),
        prompt,
        optional_params: generation_params(probe)?,
        extra_headers: extra.as_deref(),
        api_base: None,
        api_key: None,
        timeout: None,
        aimg_generation: false,
        environment: ImageEnvironment::default(),
        now: instant(NOW_SECONDS).map_err(|error| failure("Clock", error))?,
        created: NOW_SECONDS,
    };
    exchange_row(BedrockImageOperation::Generate(operation), probe).await
}

// The generation leaf reads the typed parameter record the arm already normalised
fn generation_params(probe: &Probe) -> Result<Params, Box<HelperError>> {
    let value = serde_json::to_value(Json::Object(probe.optional_params.clone()))
        .map_err(|error| failure("Params", error))?;
    serde_json::from_value(value).map_err(|error| failure("Params", error))
}
