use betterlite_bedrock::transcription::binding::{
    TranscriptionBinding, TranscriptionResponse, async_audio_transcriptions, audio_transcriptions,
    prepare_arguments,
};
use betterlite_openai::api::audio_file_input::{
    AudioFile, AudioTuple, AuthorizedAudioFiles, TupleContent,
};

use super::super::call::{TranscriptionFile, TranscriptionOperation};
use super::super::error::BedrockOtherError;

// The binding seam LiteLLM reaches with NO_PYTHON: the same prepared arguments go to sync or async
pub(super) async fn transcription<Files, B>(
    operation: TranscriptionOperation<'_, B>,
    files: &Files,
) -> Result<TranscriptionResponse, BedrockOtherError<B::Error>>
where
    Files: AuthorizedAudioFiles + Sync,
    B: TranscriptionBinding + Send + Sync,
{
    // The typed file becomes the AudioFile seam B642 takes, and it is consumed before the binding's future is sent
    let TranscriptionOperation {
        call,
        file,
        asynchronous,
        binding,
    } = operation;
    let audio = match file {
        TranscriptionFile::Bytes(bytes) => AudioFile::Bytes(bytes),
        TranscriptionFile::Multipart { filename, content } => AudioFile::Tuple(AudioTuple {
            filename,
            content: Some(TupleContent::Bytes(content)),
        }),
    };
    let arguments = prepare_arguments(audio, files, &call)?;
    if asynchronous {
        Ok(async_audio_transcriptions(&binding, &arguments).await?)
    } else {
        Ok(audio_transcriptions(&binding, &arguments)?)
    }
}
