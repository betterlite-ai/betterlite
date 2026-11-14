use std::convert::Infallible;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_bedrock::transcription::binding::{
    BindingArguments, TranscriptionBinding, TranscriptionCall, async_audio_transcriptions,
    audio_transcriptions, prepare_arguments,
};
use betterlite_bedrock::transcription::params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, TupleContent,
};
use betterlite_openai::batches::sdk::Timeout;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tokio::runtime::Builder;

use crate::helpers::{HelperError, OrHelperError as _};

use super::input::{Call, File, Mode};

struct NoFiles;

impl AuthorizedAudioFiles for NoFiles {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }

    fn open(&self, _path: &Path) -> io::Result<Box<dyn Read + Send>> {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }
}

struct Recorder {
    reply: Params,
    seen: Mutex<Vec<Value>>,
}

impl Recorder {
    fn answer(&self, arguments: &BindingArguments<'_>) -> Params {
        let call = arguments.call;
        let seen = json!({
            "model": call.model,
            "audio": arguments.audio,
            "api_key": call.api_key.map(ExposeSecret::expose_secret),
            "api_base": call.api_base,
            "custom_llm_provider": call.custom_llm_provider,
            "extra_headers": call.extra_headers,
            "optional_params": call.optional_params,
            "timeout_seconds": arguments.timeout_seconds,
        });
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
        self.reply.clone()
    }
}

#[async_trait]
impl TranscriptionBinding for Recorder {
    type Error = Infallible;

    fn transcribe(&self, arguments: &BindingArguments<'_>) -> Result<Params, Infallible> {
        Ok(self.answer(arguments))
    }

    async fn atranscribe(&self, arguments: &BindingArguments<'_>) -> Result<Params, Infallible> {
        Ok(self.answer(arguments))
    }
}

fn reply_text(
    mode: Mode,
    recorder: &Recorder,
    arguments: &BindingArguments<'_>,
) -> Result<Option<String>, Box<HelperError>> {
    let response = match mode {
        Mode::Sync => audio_transcriptions(recorder, arguments).or_value_error()?,
        Mode::Async => Builder::new_current_thread()
            .build()
            .or_value_error()?
            .block_on(async_audio_transcriptions(recorder, arguments))
            .or_value_error()?,
    };
    Ok(response.text)
}

fn reply_params(call: &Call) -> Params {
    let mut reply = Params::new();
    if let Some(text) = &call.reply_text {
        reply.insert("text".to_owned(), Json::Text(text.clone()));
    }
    reply
}

fn timeout_of(call: &Call) -> Option<Timeout> {
    match (&call.phases, call.timeout) {
        (Some(phases), _) => Some(Timeout::Phases(phases.clone())),
        (None, Some(seconds)) => Some(Timeout::Seconds(seconds)),
        (None, None) => None,
    }
}

fn audio_of(file: &File) -> AudioFile<'_> {
    match file {
        File::Bytes { content } => AudioFile::Bytes(content.as_bytes()),
        File::Tuple { filename, content } => AudioFile::Tuple(AudioTuple {
            filename,
            content: Some(TupleContent::Bytes(content.as_bytes())),
        }),
    }
}

pub(super) fn run_call(call: &Call) -> Result<Value, Box<HelperError>> {
    let recorder = Recorder {
        reply: reply_params(call),
        seen: Mutex::new(Vec::new()),
    };
    let key = call.api_key.clone().map(SecretString::from);
    let timeout = timeout_of(call);
    let source = TranscriptionCall {
        model: &call.model,
        api_key: key.as_ref(),
        api_base: call.api_base.as_deref(),
        custom_llm_provider: &call.custom_llm_provider,
        extra_headers: call.extra_headers.as_ref(),
        optional_params: &call.optional_params,
        timeout: timeout.as_ref(),
    };
    let outcome = match prepare_arguments(audio_of(&call.file), &NoFiles, &source) {
        Ok(arguments) => Ok(reply_text(call.mode, &recorder, &arguments)?),
        Err(error) => Err(error.to_string()),
    };
    let seen = recorder
        .seen
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(match outcome {
        Ok(text) => json!({"ok": true, "text": text, "seen": seen}),
        Err(error) => json!({"ok": false, "error": error, "seen": seen}),
    })
}
