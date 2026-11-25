use super::convert::fields::target;
use super::convert::{DEFAULT_IMAGE_NAME, ImageAnswer, ImageCallWire, OPENAI_PROVIDER, images};
use super::error::{CallFailure, context, convert_failure, provider_failure};
use super::session::{CallSetup, ImageSession, SilentImageLogger, call_setup, now_seconds};
use betterlite_error::error::ErrorClass;
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
        return Err(local_failure(
            format!(
                "Invalid image variation provider: {provider}. Supported providers are: LITELLM_IMAGE_VARIATION_PROVIDERS"
            ),
            &model,
            &provider,
            async_call,
        ));
    }
    let setup = call_setup(
        session,
        &call.arguments,
        &call.settings,
        &model,
        OPENAI_PROVIDER,
    )?;
    if setup.credentials.api_key.is_none() {
        return Err(local_failure(
            String::from("API key is required for OpenAI image variations"),
            &model,
            OPENAI_PROVIDER,
            async_call,
        ));
    }
    let image = first_image(&files, &model)
        .map_err(|error| local_failure(error.message, &model, OPENAI_PROVIDER, async_call))?;
    round(session, setup, image, &model, async_call).await
}

async fn round(
    session: &ImageSession,
    setup: CallSetup,
    image: ImageFile,
    model: &str,
    async_call: bool,
) -> Result<ImageAnswer, CallFailure> {
    let provider = OPENAI_PROVIDER;
    let request = ImageVariationRequest {
        model: Some(model.to_owned()),
        image,
        optional_params: RequestMap::new(),
        headers: Some(setup.headers),
        api_key: setup.credentials.api_key,
        api_base: Some(setup.credentials.api_base),
        organization: setup.credentials.organization,
        timeout: Some(setup.timeout),
        params: ImageVariationParams { async_call },
        client: None,
        shared_session: None,
        config: Some(OpenAiImageVariationConfig),
        boundary: Uuid::new_v4().to_string(),
        now_seconds: now_seconds(),
    };
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
    images(files, "image", DEFAULT_IMAGE_NAME)
        .into_iter()
        .next()
        .ok_or_else(|| {
            CallFailure::plain(
                "ValueError",
                String::from("image is required for image variations"),
                Some(model.to_owned()),
            )
        })
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

fn local_failure(message: String, model: &str, provider: &str, async_call: bool) -> CallFailure {
    if async_call {
        CallFailure::from_class(&ErrorClass::ApiConnection(Box::new(context(
            message,
            Some(model.to_owned()),
            Some(provider.to_owned()),
        ))))
    } else {
        CallFailure::plain("ValueError", message, Some(model.to_owned()))
    }
}
