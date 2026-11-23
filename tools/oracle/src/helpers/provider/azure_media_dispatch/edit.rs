use std::collections::BTreeMap;

use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_media_dispatch::AzureMediaResult;
use betterlite_provider::azure_media_dispatch::call::{AzureMediaOperation, ImageEditOperation};
use betterlite_provider::azure_media_dispatch::test_support::Reply;
use betterlite_provider::operation::OperationResult;
use secrecy::SecretString;
use uuid::Uuid;

use super::case::Input;
use super::exchange;
use super::{EDIT_REPLY, IMAGE, NOW, edit_view, outcome_error};

pub(super) async fn image_edit(input: &Input) -> HelperResult {
    let call = ImageEditOperation {
        model: input.model.clone(),
        prompt: Some(input.prompt.clone()),
        image: Some(vec![Some(ImageEditFile {
            content: IMAGE.to_vec(),
            name: None,
        })]),
        mask: None,
        options: input.options.clone(),
        headers: BTreeMap::new(),
        extra_headers: BTreeMap::new(),
        api_key: Some(SecretString::from(input.api_key.clone())),
        api_base: Some(input.api_base.clone()),
        api_version: Some(input.api_version.clone()),
        timeout: None,
        provider: None,
        params: LiteLlmParams::default(),
        boundary: Uuid::from_u128(7),
        now_seconds: NOW,
    };
    let outcome = exchange(
        AzureMediaOperation::ImageEdit(Box::new(call)),
        vec![Reply::json(200, EDIT_REPLY)],
    )
    .await?;
    let Ok(OperationResult::AzureMedia(AzureMediaResult::ImageEdit(edited))) = &outcome.result
    else {
        return outcome_error(&outcome.result);
    };
    Ok(edit_view(edited))
}
