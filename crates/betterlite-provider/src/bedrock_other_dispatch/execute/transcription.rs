use betterlite_bedrock::transcription::binding::{
    TranscriptionBinding, TranscriptionResponse, async_audio_transcriptions, audio_transcriptions,
    prepare_arguments,
};
use betterlite_openai::api::audio_file_input::AuthorizedAudioFiles;

use super::call::TranscriptionOperation;
use super::error::BedrockOtherError;

// The binding seam LiteLLM reaches with NO_PYTHON: the same prepared arguments go to sync or async
pub(super) async fn transcription<Files, B>(
    operation: TranscriptionOperation<'_, B>,
    files: &Files,
) -> Result<TranscriptionResponse, BedrockOtherError<B::Error>>
where
    Files: AuthorizedAudioFiles,
    B: TranscriptionBinding + Sync,
{
    let arguments = prepare_arguments(operation.audio, files, &operation.call)?;
    if operation.asynchronous {
        Ok(async_audio_transcriptions(&operation.binding, &arguments).await?)
    } else {
        Ok(audio_transcriptions(&operation.binding, &arguments)?)
    }
}
