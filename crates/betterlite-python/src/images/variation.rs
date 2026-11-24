use super::convert::fields::{headers, images, resolve_target, text, timeout};
use super::convert::{
    DEFAULT_GENERATION_MODEL, DEFAULT_TIMEOUT, ImageAnswer, ImageCallWire, OPENAI_PROVIDER,
};
use super::error::{CallFailure, convert_failure, provider_failure};
use super::session::{Entry, ImageSession, SilentImageLogger, explicit_credentials, now_seconds};
use betterlite_openai::api::image_contracts::records::ImageFile;
use betterlite_openai::image_variations::error::{ImageVariationSource, OpenAiImageVariationError};
use betterlite_openai::image_variations::request::{
    begin_image_variation, finish_image_variation, send_image_variation,
};
use betterlite_openai::image_variations::{
    ImageVariationParams, ImageVariationRequest, OpenAiImageVariationConfig,
};
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_files::CallFile;
use betterlite_sdk::call_setup::RequestMap;
use uuid::Uuid;

pub(crate) async fn variation(
    session: &ImageSession,
    call: ImageCallWire,
    files: Vec<(String, CallFile)>,
    entry: Entry,
) -> Result<ImageAnswer, CallFailure> {
    let requested = text(&call.arguments, "model")
        .map_err(|error| convert_failure(&error, None, None))?
        .unwrap_or_else(|| DEFAULT_GENERATION_MODEL.to_owned());
    let custom = text(&call.arguments, "custom_llm_provider")
        .map_err(|error| convert_failure(&error, Some(requested.clone()), None))?;
    let (model, provider) = resolve_target(&requested, custom.as_deref())
        .map_err(|error| convert_failure(&error, Some(requested.clone()), None))?;
    if provider != OPENAI_PROVIDER {
        return Err(CallFailure::plain(
            "ValueError",
            format!(
                "Invalid image variation provider: {provider}. Supported providers are: LITELLM_IMAGE_VARIATION_PROVIDERS"
            ),
            Some(model),
        ));
    }
    let image = first_image(&files, &model)?;
    let headers = headers(&call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?;
    let timeout = timeout(&call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?
        .unwrap_or(DEFAULT_TIMEOUT);
    let explicit = explicit_credentials(&call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?;
    let credentials = session.credentials(&call.settings, explicit);
    let Some(api_key) = credentials.api_key else {
        return Err(CallFailure::plain(
            "ValueError",
            String::from("API key is required for OpenAI image variations"),
            Some(model),
        ));
    };
    let request = ImageVariationRequest {
        model: Some(model.clone()),
        image,
        // LiteLLM's image_variations handler takes optional_params={} whatever the signature declares
        optional_params: RequestMap::new(),
        headers: Some(headers),
        api_key: Some(api_key),
        api_base: Some(credentials.api_base),
        organization: credentials.organization,
        timeout: Some(timeout),
        params: ImageVariationParams {
            async_call: entry == Entry::Async,
        },
        client: None,
        shared_session: None,
        config: Some(OpenAiImageVariationConfig),
        boundary: Uuid::new_v4().to_string(),
        now_seconds: now_seconds(),
    };
    let mut logger = SilentImageLogger;
    let prepared = begin_image_variation(request, &mut logger)
        .map_err(|error| failure(&error, &model, &provider, &session.redactor))?;
    let sent: Result<_, ImageVariationSource> =
        send_image_variation(&prepared, &session.client).await;
    let outcome = finish_image_variation(&prepared, sent, &mut logger)
        .map_err(|error| failure(&error, &model, &provider, &session.redactor))?;
    Ok(ImageAnswer::new(outcome.reply, None))
}

fn first_image(files: &[(String, CallFile)], model: &str) -> Result<ImageFile, CallFailure> {
    let mut found = images(files, "image");
    if found.is_empty() {
        return Err(CallFailure::plain(
            "ValueError",
            String::from("image is required for image variations"),
            Some(model.to_owned()),
        ));
    }
    Ok(found.remove(0))
}

fn failure(
    error: &OpenAiImageVariationError,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> CallFailure {
    provider_failure(
        error.message.clone(),
        Some(error.status_code),
        model,
        provider,
        redactor,
    )
}
