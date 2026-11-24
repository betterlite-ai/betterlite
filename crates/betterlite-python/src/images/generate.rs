use super::convert::fields::{generation_target, headers, text, timeout};
use super::convert::options::generation_options;
use super::convert::{ConvertError, DEFAULT_TIMEOUT, ImageAnswer, ImageCallWire, OPENAI_PROVIDER};
use super::error::{CallFailure, connection_failure, convert_failure, provider_failure};
use super::session::{ImageSession, SilentImageLogger, explicit_credentials, now_seconds};
use betterlite_openai::image_generation::call::ImageGenerationCall;
use betterlite_openai::image_generation::error::ImageGenerationFailure;
use betterlite_openai::image_generation::request::{
    begin_image_generation, finish_image_generation, send_image_generation,
};
use betterlite_redact::secret::Redactor;
use betterlite_sdk::call_setup::RequestMap;

pub(crate) async fn generate(
    session: &ImageSession,
    call: ImageCallWire,
) -> Result<ImageAnswer, CallFailure> {
    let (model, provider) =
        generation_target(&call.arguments).map_err(|error| convert_failure(&error, None, None))?;
    if provider != OPENAI_PROVIDER {
        return Err(connection_failure(
            format!("image generation is not supported for {provider}"),
            Some(model),
            Some(provider),
        ));
    }
    let prompt = prompt(&call.arguments, &model, &provider)?;
    let optional_params = generation_options(&call.arguments);
    let headers = headers(&call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?;
    let timeout = timeout(&call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?
        .unwrap_or(DEFAULT_TIMEOUT);
    let explicit = explicit_credentials(&call.arguments)
        .map_err(|error| convert_failure(&error, Some(model.clone()), Some(provider.clone())))?;
    let credentials = session.credentials(&call.settings, explicit);
    let request = ImageGenerationCall {
        model: model.clone(),
        prompt,
        optional_params,
        api_key: credentials.api_key,
        api_base: Some(credentials.api_base),
        organization: credentials.organization,
        headers: Some(headers),
        timeout,
        delegate_async: true,
        now_seconds: now_seconds(),
    };
    let mut logger = SilentImageLogger;
    let prepared = begin_image_generation(request, &mut logger)
        .map_err(|error| failure(&error, &model, &provider, &session.redactor))?;
    let sent = send_image_generation(&prepared, &session.client).await;
    let outcome = finish_image_generation(&prepared, sent, &mut logger)
        .map_err(|error| failure(&error, &model, &provider, &session.redactor))?;
    Ok(ImageAnswer::new(outcome.reply, Some(&model)))
}

fn prompt(arguments: &RequestMap, model: &str, provider: &str) -> Result<String, CallFailure> {
    match text(arguments, "prompt") {
        Ok(Some(prompt)) => Ok(prompt),
        Ok(None) | Err(_) => Err(convert_failure(
            &ConvertError::Field {
                name: String::from("prompt"),
                expected: "text",
            },
            Some(model.to_owned()),
            Some(provider.to_owned()),
        )),
    }
}

fn failure(
    error: &ImageGenerationFailure,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> CallFailure {
    provider_failure(error.to_string(), error.status(), model, provider, redactor)
}
