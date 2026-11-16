use betterlite_azure::azure_urls::url_parts::encoded_query;
use betterlite_config::sections::foundry_image_edits::Settings as VersionSettings;
use betterlite_openai::openai_base_config::overlapping_api_base_path;

use crate::credentials::model_info::ModelInfoAccessors;
use crate::image_generation::flux::flux2_image_generation_url;
use crate::mai_images::url::{ImageOperation, mai_image_url};

use super::error::ImageEditError;
use super::selection::ImageEditVariant;

pub struct CompleteUrlRequest<'data> {
    pub model: &'data str,
    pub accessors: ModelInfoAccessors<'data>,
    pub api_base: Option<&'data str>,
    pub api_version: Option<&'data str>,
    pub global_api_version: Option<&'data str>,
    pub environment: &'data VersionSettings,
}

// FLUX 1 requires a version, FLUX 2 defaults to preview, and MAI never reads litellm.api_version, matching its config
pub fn complete_url(request: CompleteUrlRequest<'_>) -> Result<String, ImageEditError> {
    let variant = ImageEditVariant::select(request.model)?;
    let base = request
        .accessors
        .api_base(request.api_base)
        .ok_or(ImageEditError::ApiBaseRequired)?;
    let environment_version = request.environment.api_version.as_deref();
    match variant {
        ImageEditVariant::Flux1 => {
            let version = first_version(&[
                request.api_version,
                request.global_api_version,
                environment_version,
            ])
            .ok_or(ImageEditError::ApiVersionRequired)?;
            flux1_edit_url(&base, request.model, version)
        }
        ImageEditVariant::Flux2 => {
            let version = first_version(&[
                request.api_version,
                request.global_api_version,
                environment_version,
            ]);
            Ok(flux2_image_generation_url(
                Some(base.as_str()),
                request.model,
                version,
            )?)
        }
        ImageEditVariant::Mai => {
            let version = first_version(&[request.api_version, environment_version]);
            Ok(mai_image_url(
                ImageOperation::Edit,
                Some(base.as_str()),
                version,
            )?)
        }
    }
}

fn first_version<'data>(sources: &[Option<&'data str>]) -> Option<&'data str> {
    sources.iter().find_map(|source| *source)
}

// B668 appends /images/edits to a deployment base, and the caller's query, if any, is replaced by the api-version alone
fn flux1_edit_url(
    api_base: &str,
    model: &str,
    api_version: &str,
) -> Result<String, ImageEditError> {
    let ending = if api_base.contains("/openai/deployments/") {
        String::from("/images/edits")
    } else {
        format!("/openai/deployments/{model}/images/edits")
    };
    let appended = overlapping_api_base_path(api_base, &ending)?;
    Ok(with_api_version(&appended, api_version))
}

fn with_api_version(url: &str, api_version: &str) -> String {
    let path = url.split_once('?').map_or(url, |(before, _)| before);
    let pairs = [("api-version".to_owned(), api_version.to_owned())];
    encoded_query(&pairs).map_or_else(|| path.to_owned(), |query| format!("{path}?{query}"))
}
