use std::collections::BTreeMap;

use betterlite_azure::image_transport::error::ImageTransportError;
use betterlite_azure::image_transport::{ImageRequest, ImageTransport};
use betterlite_azure::images::edit::{
    EditAuth, EditParams, EditUrl, finalize_edit_data, get_complete_url, validate_environment,
};
use betterlite_http::proxy::multipart::encode;
use betterlite_openai::api::image_contracts::records::{ImageFiles, ImageParams};
use betterlite_openai::image_edits::family::ImageEditFamily;
use betterlite_openai::image_edits::file::multipart_parts;
use betterlite_openai::image_edits::request::{
    ImageEditForm, ImageEditRequest, transform_image_edit_request,
};
use betterlite_openai::image_edits::response::transform_image_edit_response;
use http::Method;
use uuid::Uuid;

use super::ImageEditOutcome;
use super::call::ImageEditOperation;
use super::error::AzureMediaError;
use super::parts::{form_parts, header_map, lossy_text};
use super::runtime::MediaRuntime;

// AzureImageEditConfig is OpenAIImageEditConfig with Azure's auth and URL, so the form always names the image[] file field.
pub(super) async fn image_edit<C, K, T, L>(
    operation: ImageEditOperation,
    runtime: &MediaRuntime<'_, C, K, T, L>,
) -> Result<ImageEditOutcome, AzureMediaError>
where
    C: Sync,
    K: Sync,
    T: ImageTransport,
    L: Sync,
{
    let boundary = operation.boundary;
    let now_seconds = operation.now_seconds;
    let (url, headers, data, files) = prepared_edit(operation, runtime)?;
    let body = edit_body(boundary, &data, &files)?;
    let sent = runtime
        .transport
        .send(ImageRequest::new(
            Method::POST,
            url.clone(),
            header_map(&headers)?,
            body,
        ))
        .await
        .map_err(|source| AzureMediaError::Transport(ImageTransportError::Transport(source)))?;
    if !(200..300).contains(&sent.status) {
        return Err(AzureMediaError::EditStatus {
            status: sent.status,
            message: lossy_text(&sent.body),
        });
    }
    let reply = transform_image_edit_response(sent.status, &sent.body, now_seconds)?;
    Ok(ImageEditOutcome {
        url,
        headers,
        data,
        files,
        reply,
    })
}

// The URL, auth headers and form the edit resolves before its multipart body is encoded
fn prepared_edit<C, K, T, L>(
    operation: ImageEditOperation,
    runtime: &MediaRuntime<'_, C, K, T, L>,
) -> Result<(String, BTreeMap<String, String>, ImageParams, ImageFiles), AzureMediaError> {
    let request = ImageEditRequest {
        model: operation.model.clone(),
        prompt: operation.prompt,
        image: operation.image,
        mask: operation.mask,
        options: operation.options,
    };
    let ImageEditForm { data, files } =
        transform_image_edit_request(ImageEditFamily::OpenAi, &request)?;
    let edit_params = EditParams {
        api_key: None,
        azure_ad_token: operation.params.azure_ad_token,
        auth: operation.params.auth,
    };
    let mut validated = validate_environment(&EditAuth {
        headers: operation.headers,
        api_key: operation.api_key.as_ref(),
        params: &edit_params,
        credentials: runtime.credentials,
        environment: runtime.environment,
        sources: runtime.sources,
        refresh_enabled: runtime.refresh_enabled,
        provider: operation.provider,
    })?;
    // the call's extra_headers update the validated headers, so a caller header wins over the auth header
    validated.extend(operation.extra_headers);
    let url = get_complete_url(&EditUrl {
        model: &operation.model,
        api_base: operation.api_base.as_deref(),
        api_version: operation.api_version.as_deref(),
        credentials: runtime.credentials,
        default_api_version: &runtime.endpoints.azure_default_api_version,
    })?;
    let finalized = finalize_edit_data(&data, &url);
    Ok((url, validated, finalized, files))
}

fn edit_body(
    boundary: Uuid,
    data: &ImageParams,
    files: &ImageFiles,
) -> Result<Vec<u8>, AzureMediaError> {
    let parts = form_parts(data)?
        .into_iter()
        .chain(multipart_parts(files))
        .collect::<Vec<_>>();
    encode(&boundary.to_string(), &parts)
        .map_err(|source| AzureMediaError::Transport(ImageTransportError::Transport(source)))
}
