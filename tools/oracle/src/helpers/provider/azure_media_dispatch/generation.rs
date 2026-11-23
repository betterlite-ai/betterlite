use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_media_dispatch::call::{
    AzureMediaOperation, ImageGenerationOperation,
};
use betterlite_provider::azure_media_dispatch::test_support::exchange;
use secrecy::SecretString;

use super::case::Input;
use super::{NOW, generation_result, json_reply};
use crate::openai_media_dispatch::failure;

pub(super) async fn image_generation(input: &Input) -> HelperResult {
    let reply = json_reply(input)?;
    let call = ImageGenerationOperation {
        model: Some(input.model.clone()),
        prompt: input.prompt.clone(),
        timeout: Some(600.0_f64),
        optional_params: input.options.clone(),
        headers: Vec::new(),
        api_key: Some(SecretString::from(input.api_key.clone())),
        api_base: Some(input.api_base.clone()),
        api_version: Some(input.api_version.clone()),
        azure_ad_token: None,
        provider: None,
        base_model: None,
        params: LiteLlmParams::default(),
        delegate_async: false,
        now_seconds: NOW,
    };
    let outcome = exchange(
        AzureMediaOperation::ImageGeneration(Box::new(call)),
        vec![reply],
    )
    .await
    .map_err(|error| failure("RuntimeError", error))?;
    generation_result(&outcome)
}
