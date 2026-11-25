use super::convert::fields::target;
use super::convert::{DEFAULT_IMAGE_NAME, ImageAnswer, ImageCallWire, OPENAI_PROVIDER, images};
use super::error::{CallFailure, convert_failure, provider_failure};
use super::session::{ImageSession, SilentImageLogger, call_setup, now_seconds};
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
    async_call: bool,
) -> Result<ImageAnswer, CallFailure> {
    let (model, provider) =
        target(&call.arguments).map_err(|error| convert_failure(&error, None, None))?;
    if provider != OPENAI_PROVIDER {
        return Err(CallFailure::plain(
            "ValueError",
            format!(
                "Invalid image variation provider: {provider}. Supported providers are: LITELLM_IMAGE_VARIATION_PROVIDERS"
            ),
            Some(model.to_owned()),
        ));
    }
    let request = request(session, &call, &files, async_call, &model)?;
    round(session, request, &model, &provider).await
}

fn request(
    session: &ImageSession,
    call: &ImageCallWire,
    files: &[(String, CallFile)],
    async_call: bool,
    model: &str,
) -> Result<ImageVariationRequest, CallFailure> {
    let setup = call_setup(
        session,
        &call.arguments,
        &call.settings,
        model,
        OPENAI_PROVIDER,
    )?;
    let Some(api_key) = setup.credentials.api_key else {
        return Err(CallFailure::plain(
            "ValueError",
            String::from("API key is required for OpenAI image variations"),
            Some(model.to_owned()),
        ));
    };
    Ok(ImageVariationRequest {
        model: Some(model.to_owned()),
        image: first_image(files, model)?,
        optional_params: RequestMap::new(),
        headers: Some(setup.headers),
        api_key: Some(api_key),
        api_base: Some(setup.credentials.api_base),
        organization: setup.credentials.organization,
        timeout: Some(setup.timeout),
        params: ImageVariationParams { async_call },
        client: None,
        shared_session: None,
        config: Some(OpenAiImageVariationConfig),
        boundary: Uuid::new_v4().to_string(),
        now_seconds: now_seconds(),
    })
}

async fn round(
    session: &ImageSession,
    request: ImageVariationRequest,
    model: &str,
    provider: &str,
) -> Result<ImageAnswer, CallFailure> {
    let redactor = &session.redactor;
    let mut logger = SilentImageLogger;
    let prepared = begin_image_variation(request, &mut logger)
        .map_err(|error| failure(&error, model, provider, redactor))?;
    let sent: Result<_, ImageVariationSource> =
        send_image_variation(&prepared, &session.client).await;
    let outcome = finish_image_variation(&prepared, sent, &mut logger)
        .map_err(|error| failure(&error, model, provider, redactor))?;
    Ok(ImageAnswer::new(outcome.reply, None))
}

pub(crate) fn first_image(
    files: &[(String, CallFile)],
    model: &str,
) -> Result<ImageFile, CallFailure> {
    let mut found = images(files, "image", DEFAULT_IMAGE_NAME);
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

mod tests;
