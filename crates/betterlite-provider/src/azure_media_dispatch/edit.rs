use betterlite_azure::image_transport::error::ImageTransportError;
use betterlite_azure::image_transport::{ImageRequest, ImageTransport};
use betterlite_azure::images::edit::{
    EditAuth, EditParams, EditUrl, finalize_edit_data, get_complete_url, validate_environment,
};
use betterlite_http::proxy::multipart::encode;
use betterlite_openai::image_edits::family::ImageEditFamily;
use betterlite_openai::image_edits::request::{ImageEditRequest, transform_image_edit_request};
use betterlite_openai::image_edits::response::transform_image_edit_response;
use http::Method;

use super::call::ImageEditOperation;
use super::error::AzureMediaError;
use super::parts::{file_parts, form_parts, header_map, lossy_text, wire_parts};
use super::runtime::MediaRuntime;
use super::{AzureMediaPrepared, ImageEditOutcome, MediaPart};

// AzureImageEditConfig is OpenAIImageEditConfig with Azure's auth and URL, so the form always names the image[] file field.
pub(super) async fn image_edit<C, K, T, L>(
    operation: ImageEditOperation,
    runtime: &MediaRuntime<'_, C, K, T, L>,
) -> Result<ImageEditOutcome, AzureMediaError>
where
    T: ImageTransport,
{
    let ImageEditOperation {
        model,
        prompt,
        image,
        mask,
        options,
        headers,
        extra_headers,
        api_key,
        api_base,
        api_version,
        provider,
        params,
        boundary,
        now_seconds,
    } = operation;
    let request = ImageEditRequest {
        model: model.clone(),
        prompt,
        image,
        mask,
        options,
    };
    let form = transform_image_edit_request(ImageEditFamily::OpenAi, &request)?;
    let edit_params = EditParams {
        api_key: None,
        azure_ad_token: params.azure_ad_token.clone(),
        auth: params.auth.clone(),
    };
    let mut validated = validate_environment(&EditAuth {
        headers,
        api_key: api_key.as_ref(),
        params: &edit_params,
        credentials: runtime.credentials,
        environment: runtime.environment,
        sources: runtime.sources,
        refresh_enabled: runtime.refresh_enabled,
        provider,
    })?;
    // the call's extra_headers update the validated headers, so a caller header wins over the auth header
    validated.extend(extra_headers);
    let url = get_complete_url(&EditUrl {
        model: &model,
        api_base: api_base.as_deref(),
        api_version: api_version.as_deref(),
        credentials: runtime.credentials,
        default_api_version: &runtime.endpoints.azure_default_api_version,
    })?;
    let data = finalize_edit_data(&form.data, &url);
    let parts: Vec<MediaPart> = form_parts(&data)?
        .into_iter()
        .chain(file_parts(&form.files))
        .collect();
    let body = encode(&boundary.to_string(), &wire_parts(&parts))
        .map_err(|source| AzureMediaError::Transport(ImageTransportError::Transport(source)))?;
    let sent = runtime
        .transport
        .send(ImageRequest::new(
            Method::POST,
            url.clone(),
            header_map(&validated)?,
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
        request: AzureMediaPrepared {
            method: Method::POST,
            url,
            headers: validated,
            parts,
        },
        reply,
    })
}
