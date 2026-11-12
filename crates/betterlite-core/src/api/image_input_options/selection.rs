use thiserror::Error;

use super::options::ImageEditOptionalRequestParams;

pub const UNSUPPORTED_PARAMS_STATUS: u16 = 400;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ImageInputOptionsError {
    #[error("The following parameters are not supported for model {model}: {params}")]
    UnsupportedParams { model: String, params: String },
}

pub trait ImageEditProviderConfig {
    type Error: From<ImageInputOptionsError>;
    type Mapped;

    fn supported_openai_params(&self, model: &str) -> Vec<String>;

    fn map_openai_params(
        &self,
        options: &ImageEditOptionalRequestParams,
        model: &str,
        drop_params: bool,
    ) -> Result<Self::Mapped, Self::Error>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OptionalParamsPolicy {
    pub global_drop_params: bool,
    pub call_drop_params: bool,
    pub additional_drop_params: Vec<String>,
}

#[must_use]
pub fn requested_optional_params(
    params: &ImageEditOptionalRequestParams,
    provider_supported_params: &[String],
) -> ImageEditOptionalRequestParams {
    let mut selected = params.clone();
    selected.extensions.retain(|name, value| {
        !value.is_null()
            && provider_supported_params
                .iter()
                .any(|supported| supported == name)
    });
    selected
}

pub fn optional_params_image_edit<P>(
    provider: &P,
    model: &str,
    options: &ImageEditOptionalRequestParams,
    policy: &OptionalParamsPolicy,
) -> Result<P::Mapped, P::Error>
where
    P: ImageEditProviderConfig,
{
    let supported = provider.supported_openai_params(model);
    let mut filtered = options.clone();
    for name in &policy.additional_drop_params {
        filtered.remove(name);
    }
    let unsupported = filtered
        .present_names()
        .into_iter()
        .filter(|name| {
            !supported
                .iter()
                .any(|supported_name| supported_name == name)
        })
        .collect::<Vec<_>>();
    let drop_params = policy.global_drop_params || policy.call_drop_params;
    if !unsupported.is_empty() && !drop_params {
        return Err(P::Error::from(ImageInputOptionsError::UnsupportedParams {
            model: model.to_owned(),
            params: unsupported.join(", "),
        }));
    }
    for name in &unsupported {
        filtered.remove(name);
    }
    provider.map_openai_params(&filtered, model, drop_params)
}
