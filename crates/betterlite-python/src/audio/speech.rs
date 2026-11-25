use super::convert::speech::{SpeechValues, speech_values};
use super::convert::{
    DEFAULT_MAX_RETRIES, OPENAI_PROVIDER, SpeechAnswer, SpeechCallWire, SpeechHeader, text,
};
use super::error::{CallFailure, context, missing_key, provider_failure};
use super::session::{AudioSession, SilentAudioLogger};
use betterlite_error::error::ErrorClass;
use betterlite_openai::speech::call::{SpeechCall, audio_speech};
use betterlite_openai::speech::client::SpeechConnection;
use betterlite_openai::speech::error::SpeechError;
use betterlite_openai::speech::request::SpeechRequest;
use betterlite_redact::secret::Redactor;

use crate::images::convert::fields::resolve_target;

struct Prepared {
    model: String,
    provider: String,
    values: SpeechValues,
    voice: String,
    connection: SpeechConnection,
}

pub(crate) async fn run(
    session: &AudioSession,
    call: SpeechCallWire,
) -> Result<SpeechAnswer, CallFailure> {
    let prepared = prepare(session, &call)?;
    let request = SpeechRequest {
        model: &prepared.model,
        input: &prepared.values.input,
        voice: &prepared.voice,
        options: &prepared.values.options,
    };
    let speech = SpeechCall {
        request,
        connection: prepared.connection,
        aspeech: call.aspeech,
    };
    let mut logger = SilentAudioLogger;
    let pending = audio_speech(speech, &mut logger, &session.client).map_err(|error| {
        speech_failure(
            error,
            &prepared.model,
            &prepared.provider,
            &session.redactor,
        )
    })?;
    let response = pending.await.map_err(|error| {
        speech_failure(
            error,
            &prepared.model,
            &prepared.provider,
            &session.redactor,
        )
    })?;
    let header = SpeechHeader {
        status_code: response.status,
        headers: response.headers.clone(),
    };
    let content = response
        .content(session.client.limits.body_bytes)
        .await
        .map_err(|error| {
            speech_failure(
                error,
                &prepared.model,
                &prepared.provider,
                &session.redactor,
            )
        })?;
    Ok(SpeechAnswer { content, header })
}

fn prepare(session: &AudioSession, call: &SpeechCallWire) -> Result<Prepared, CallFailure> {
    let values = speech_values(&call.arguments)?;
    let provider_arg = text(&call.arguments, "custom_llm_provider")?;
    let (model, provider) = resolve_target(&values.model, provider_arg.as_deref());
    if provider != OPENAI_PROVIDER {
        return Err(CallFailure::plain(
            "Exception",
            format!("Unable to map the custom llm provider={provider} to a known provider."),
            None,
        ));
    }
    let Some(voice) = values.voice.clone() else {
        return Err(missing_voice(&model, &provider));
    };
    let connection = speech_connection(session, call, &values)?;
    Ok(Prepared {
        connection,
        model,
        provider,
        values,
        voice,
    })
}

fn speech_connection(
    session: &AudioSession,
    call: &SpeechCallWire,
    values: &SpeechValues,
) -> Result<SpeechConnection, CallFailure> {
    let credentials = session.credentials(&call.settings, &call.arguments)?;
    let api_key = credentials
        .api_key
        .clone()
        .ok_or_else(|| missing_key(&values.model))?;
    Ok(SpeechConnection {
        api_key: Some(api_key),
        api_base: Some(credentials.api_base),
        timeout: values.timeout.unwrap_or_else(|| session.request_timeout()),
        max_retries: values
            .max_retries
            .or(call.settings.num_retries)
            .unwrap_or(DEFAULT_MAX_RETRIES),
        organization: None,
        project: None,
        environment_organization: credentials.organization,
        environment_project: credentials.project,
        client: None,
        shared_session: None,
    })
}

fn speech_failure(
    error: SpeechError,
    model: &str,
    provider: &str,
    redactor: &Redactor,
) -> CallFailure {
    match &error {
        SpeechError::Status(found) => provider_failure(
            error.to_string(),
            Some(found.status),
            model,
            provider,
            redactor,
        ),
        SpeechError::Transport { .. } => {
            provider_failure(error.to_string(), None, model, provider, redactor)
        }
        SpeechError::MissingApiKey => missing_key(model),
        SpeechError::BaseUrl { .. }
        | SpeechError::Header { .. }
        | SpeechError::Encode { .. }
        | SpeechError::ClientKind { .. }
        | SpeechError::Logging { .. }
        | SpeechError::Body { .. }
        | SpeechError::TooLarge => {
            CallFailure::plain("ValueError", error.to_string(), Some(model.to_owned()))
        }
    }
}

fn missing_voice(model: &str, provider: &str) -> CallFailure {
    CallFailure::from_class(&ErrorClass::BadRequest(Box::new(context(
        String::from("'voice' is required to be passed as a string for OpenAI TTS"),
        Some(model.to_owned()),
        Some(provider.to_owned()),
    ))))
}
