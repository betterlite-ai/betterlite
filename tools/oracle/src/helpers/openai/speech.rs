mod fixture;
mod input;
mod tests;

use std::fmt::Display;

use betterlite_openai::speech::call::{SpeechCall, audio_speech};
use betterlite_openai::speech::client::{ClientKind, select_client};
use betterlite_openai::speech::error::SpeechError;
use betterlite_openai::speech::request::SpeechRequest;
use betterlite_openai::speech::result::SpeechResponse;
use input::Input;
use secrecy::ExposeSecret as _;
use serde_json::Value;
use serde_json::json;

use self::fixture::{Fixture, Recorder, logging_view, request_view};
use crate::helpers::{Helper, HelperError, HelperResult};

const BODY_LIMIT: usize = 1024 * 1024;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let fixture = Fixture::new(input.reply.as_ref()).map_err(failure)?;
        let mut recorder = Recorder::default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .map_err(failure)?;
        let outcome = runtime.block_on(exercise(input, &fixture, &mut recorder));
        Ok(json!({
            "client": client_view(input).unwrap_or_else(|error| failure_view(&error)),
            "request": request_view(&fixture, &input.request.extra_headers),
            "result": outcome.unwrap_or_else(|error| failure_view(&error)),
            "logging": logging_view(&recorder),
        }))
    }
}

async fn exercise(
    input: &Input,
    fixture: &Fixture,
    recorder: &mut Recorder,
) -> Result<Value, SpeechError> {
    let options = input::options(&input.request);
    let call = SpeechCall {
        request: SpeechRequest {
            model: &input.request.model,
            input: &input.request.input,
            voice: &input.request.voice,
            options: &options,
        },
        connection: input::connection(&input.client)?,
        aspeech: input.client.aspeech,
    };
    match audio_speech(call, recorder, fixture).await {
        Ok(response) => result_view(response, BODY_LIMIT).await,
        Err(error) => Ok(failure_view(&error)),
    }
}

async fn result_view(response: SpeechResponse, limit: usize) -> Result<Value, SpeechError> {
    let status = response.status;
    let content_type = response.content_type.clone();
    let headers = response.headers.clone();
    let additional_headers = response.additional_headers.clone();
    let body = response.content(limit).await?;
    Ok(json!({
        "status": status,
        "content_type": content_type,
        "headers": headers,
        "additional_headers": additional_headers,
        "body": body,
    }))
}

fn client_view(input: &Input) -> Result<Value, SpeechError> {
    let connection = input::connection(&input.client)?;
    let kind = if input.client.aspeech == Some(true) {
        ClientKind::Async
    } else {
        ClientKind::Sync
    };
    let client = select_client(&connection, kind)?;
    let key_is_given = client
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().as_str())
        == input.client.api_key.as_deref();
    Ok(json!({
        "kind": client.kind.label(),
        "base_url": client.base_url.as_str(),
        "timeout": client.timeout.as_secs(),
        "max_retries": client.max_retries,
        "organization": client.organization,
        "key_is_given": key_is_given,
    }))
}

fn failure_view(error: &SpeechError) -> Value {
    if let SpeechError::Status(status) = error {
        return json!({
            "raised": {
                "status": status.status,
                "type": status.error_type,
                "param": status.param,
                "code": status.code,
            }
        });
    }
    json!({"raised": {"kind": "error", "message": error.to_string()}})
}

fn failure<M>(message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: String::from("OpenAiSpeechError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
