use super::convert::fields::{target, text};
use super::convert::options::generation_options;
use super::convert::{ConvertError, ImageAnswer, ImageCallWire, OPENAI_PROVIDER};
use super::error::{CallFailure, connection_failure, convert_failure, provider_failure};
use super::session::{ImageSession, SilentImageLogger, call_setup, now_seconds};
use betterlite_openai::image_generation::call::ImageGenerationCall;
use betterlite_openai::image_generation::reply::ImageGenerationReply;
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
        return Err(connection_failure(
            format!("image generation is not supported for {provider}"),
            Some(model),
            Some(provider),
        ));
    }
    let request = request(session, &call, &model, &provider)?;
    let reply = round(session, request, &model, &provider).await?;
    Ok(ImageAnswer::new(reply, Some(&model)))
}

fn request(
    session: &ImageSession,
    call: &ImageCallWire,
    model: &str,
    provider: &str,
) -> Result<ImageGenerationCall, CallFailure> {
    let setup = call_setup(session, &call.arguments, &call.settings, model, provider)?;
    let prompt = match text(&call.arguments, "prompt") {
        Ok(Some(prompt)) => prompt,
        Ok(None) | Err(_) => {
            return Err(convert_failure(
                &ConvertError::Field {
                    name: String::from("prompt"),
                    expected: "text",
                },
                Some(model.to_owned()),
                Some(provider.to_owned()),
            ));
        }
    };
    let drop_params = call.settings.drop_params.unwrap_or(false);
    Ok(ImageGenerationCall {
        model: model.to_owned(),
        prompt,
        optional_params: generation_options(&call.arguments, model, drop_params)?,
        api_key: setup.credentials.api_key,
        api_base: Some(setup.credentials.api_base),
        organization: setup.credentials.organization,
        headers: Some(setup.headers),
        timeout: setup.timeout,
        delegate_async: true,
        now_seconds: now_seconds(),
    })
}

async fn round(
    session: &ImageSession,
    request: ImageGenerationCall,
    model: &str,
    provider: &str,
) -> Result<ImageGenerationReply, CallFailure> {
    let redactor = &session.redactor;
    let mut logger = SilentImageLogger;
    let prepared = begin_image_generation(request, &mut logger).map_err(|error| {
        provider_failure(error.to_string(), error.status(), model, provider, redactor)
    })?;
    let sent = send_image_generation(&prepared, &session.client).await;
    let outcome = finish_image_generation(&prepared, sent, &mut logger).map_err(|error| {
        provider_failure(error.to_string(), error.status(), model, provider, redactor)
    })?;
    Ok(outcome.reply)
}
