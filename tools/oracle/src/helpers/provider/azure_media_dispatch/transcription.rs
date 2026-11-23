use std::time::Duration;

use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::azure_media_dispatch::AzureMediaResult;
use betterlite_provider::azure_media_dispatch::call::{
    AzureMediaOperation, TranscriptionOperation,
};
use betterlite_provider::azure_media_dispatch::test_support::exchange;
use betterlite_provider::operation::OperationResult;
use indexmap::IndexMap;
use secrecy::SecretString;
use uuid::Uuid;

use super::case::Input;
use super::{
    AUDIO, AUDIO_FILE, AUDIO_TYPE, first_request, json_reply, outcome_error, transcription_view,
};
use crate::openai_media_dispatch::failure;

pub(super) async fn transcription(input: &Input) -> HelperResult {
    let call = TranscriptionOperation {
        input: String::from(AUDIO_FILE),
        file: audio_file(),
        boundary: Uuid::from_u128(11),
        model: input.model.clone(),
        optional_params: IndexMap::new(),
        atranscription: false,
        api_version: Some(input.api_version.clone()),
        api_key: Some(SecretString::from(input.api_key.clone())),
        api_base: Some(input.api_base.clone()),
        azure_ad_token: None,
        max_retries: None,
        timeout: Duration::from_secs(30),
        params: LiteLlmParams::default(),
        provider: None,
        supplied: None,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
    };
    let reply = json_reply(input)?.with_header("content-type", "application/json");
    let outcome = exchange(
        AzureMediaOperation::Transcription(Box::new(call)),
        vec![reply],
    )
    .await
    .map_err(|error| failure("RuntimeError", error))?;
    let Ok(OperationResult::AzureMedia(AzureMediaResult::Transcription(transcribed))) =
        &outcome.result
    else {
        return outcome_error(&outcome.result);
    };
    let sent = first_request(outcome.fixture.outbound())?;
    transcription_view(transcribed, &sent)
}

fn audio_file() -> ProcessedAudioFile {
    ProcessedAudioFile {
        file_content: AUDIO.to_vec(),
        filename: String::from(AUDIO_FILE),
        content_type: String::from(AUDIO_TYPE),
    }
}
