use super::convert::fields::{field, target, text};
use super::convert::options::generation_options;
use super::convert::{ImageAnswer, ImageCallWire, OPENAI_PROVIDER};
use super::error::{CallFailure, context, convert_failure, provider_failure};
use super::session::{ImageSession, SilentImageLogger, call_setup, now_seconds};
use betterlite_error::error::ErrorClass;
use betterlite_openai::image_generation::call::ImageGenerationCall;
use betterlite_openai::image_generation::request::{
    begin_image_generation, finish_image_generation, send_image_generation,
};

pub(crate) async fn generate(
    session: &ImageSession,
    call: ImageCallWire,
) -> Result<ImageAnswer, CallFailure> {
    let (model, provider) =
        target(&call.arguments).map_err(|error| convert_failure(&error, None, None))?;
    if provider != OPENAI_PROVIDER {
        let message = format!("image generation is not supported for {provider}");
        return Err(CallFailure::from_class(
            ErrorClass::ApiConnection(Box::new(context(
                message.clone(),
                Some(model),
                Some(provider),
            ))),
            message,
        ));
    }
    let request = request(session, &call, &model, &provider)?;
    let mut logger = SilentImageLogger;
    let prepared = begin_image_generation(request, &mut logger).map_err(|error| {
        provider_failure(
            error.to_string(),
            error.status(),
            &model,
            &provider,
            &session.redactor,
        )
    })?;
    let sent = send_image_generation(&prepared, &session.client).await;
    let outcome = finish_image_generation(&prepared, sent, &mut logger).map_err(|error| {
        provider_failure(
            error.to_string(),
            error.status(),
            &model,
            &provider,
            &session.redactor,
        )
    })?;
    let mut answer = ImageAnswer::new(outcome.reply, Some(&model));
    answer.set_headers(outcome.provider_headers);
    Ok(answer)
}

pub(crate) fn request(
    session: &ImageSession,
    call: &ImageCallWire,
    model: &str,
    provider: &str,
) -> Result<ImageGenerationCall, CallFailure> {
    let setup = call_setup(session, &call.arguments, &call.settings, model, provider)?;
    let Some(prompt) = text(&call.arguments, "prompt").unwrap_or_default() else {
        return Err(convert_failure(
            &field("prompt", "text"),
            Some(model.to_owned()),
            Some(provider.to_owned()),
        ));
    };
    let options = generation_options(
        &call.arguments,
        model,
        call.settings.drop_params.unwrap_or(false),
    )
    .map_err(|error| convert_failure(&error, Some(model.to_owned()), Some(provider.to_owned())))?;
    Ok(ImageGenerationCall {
        model: model.to_owned(),
        prompt,
        optional_params: options,
        api_key: setup.credentials.api_key,
        api_base: Some(setup.credentials.api_base),
        organization: setup.credentials.organization,
        headers: Some(setup.headers),
        timeout: setup.timeout,
        delegate_async: true,
        now_seconds: now_seconds(),
    })
}

mod tests;
