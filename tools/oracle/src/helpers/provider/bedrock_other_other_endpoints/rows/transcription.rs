use std::error::Error as StdError;

use betterlite_bedrock::transcription::binding::TranscriptionCall;
use betterlite_bedrock::transcription::error::CallError;
use betterlite_bedrock::transcription::params::Params;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{AudioFile, AudioTuple, TupleContent};
use betterlite_openai::batches::sdk::Timeout as OpenAiTimeout;
use betterlite_provider::bedrock_other_dispatch::BedrockOtherResult;
use betterlite_provider::bedrock_other_dispatch::call::{
    BedrockOtherOperation, TranscriptionOperation,
};
use betterlite_provider::bedrock_other_dispatch::test_support::binding::Recording;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::super::harness::{Harness, Outcome, exchange};
use super::super::model::{Failed, File, Probe, Unavailable, from_params};
use super::row;
use crate::openai_media_dispatch::failure;

const UNSUPPORTED_PREFIX: &str = "Unsupported Bedrock audio format";

pub(super) async fn transcription_row(probe: &Probe) -> Result<Value, Failed> {
    let file = probe
        .file
        .as_ref()
        .ok_or_else(|| failure("File", "a transcription probe carries a file"))?;
    let params: Params = from_params(probe)?;
    let headers = extra_headers(probe);
    let timeout = probe.timeout.map(OpenAiTimeout::Seconds);
    let key = probe.api_key.clone().map(SecretString::from);
    let text = probe.reply_text.clone().unwrap_or_default();
    let recording = Recording::new(&text);
    let observer = recording.clone();
    let call = TranscriptionCall {
        model: &probe.model,
        api_key: key.as_ref(),
        api_base: probe.api_base.as_deref(),
        custom_llm_provider: "bedrock",
        extra_headers: headers.as_ref(),
        optional_params: &params,
        timeout: timeout.as_ref(),
    };
    let operation = BedrockOtherOperation::Transcription(TranscriptionOperation {
        call,
        audio: audio_file(file),
        asynchronous: probe.asynchronous,
        binding: recording,
    });
    let harness = Harness::new(probe)?;
    let outcome: Outcome<CallError> =
        exchange::<Unavailable, Recording>(&harness, operation).await?;
    let value = match &outcome.result {
        Ok(BedrockOtherResult::Transcription(response)) => json!({
            "text": &response.text,
            "seen": observer.seen.values(),
        }),
        Ok(_other) => Value::Null,
        Err(_reported) => Value::Null,
    };
    row(&outcome, value, error_kind(&outcome))
}

fn extra_headers(probe: &Probe) -> Option<Params> {
    probe.extra_headers.as_ref().map(|values| {
        values
            .iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect()
    })
}

// The native call receives the typed file; the leaf names its format from the filename
fn audio_file<'call>(file: &'call File) -> AudioFile<'call> {
    let bytes = file.content.as_bytes();
    match &file.filename {
        Some(filename) => AudioFile::Tuple(AudioTuple {
            filename,
            content: Some(TupleContent::Bytes(bytes)),
        }),
        None => AudioFile::Bytes(bytes),
    }
}

// An unsupported container is refused by name before the binding runs; any other failure is the binding's
fn error_kind<E>(outcome: &Outcome<E>) -> Option<&'static str>
where
    E: StdError + 'static,
{
    match &outcome.result {
        Ok(_ok) => None,
        Err(reported) if reported.to_string().starts_with(UNSUPPORTED_PREFIX) => {
            Some("unsupported_format")
        }
        Err(_reported) => Some("binding"),
    }
}
