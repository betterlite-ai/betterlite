use betterlite_azure::image_transport::ImageClock;
use betterlite_azure::image_transport::ImageTransport;
use betterlite_azure::image_transport::request::ImageClientConstructor;
use betterlite_azure::transcription::call::logging::AzureTranscriptionLogger;
use betterlite_openai::image_generation::logging::ImageGenerationLogger;
use betterlite_openai::transcription::transport::TranscriptionTransport;

use crate::operation::{OperationRequest, OperationResult};

use super::call::AzureMediaOperation;
use super::error::AzureMediaError;
use super::runtime::MediaRuntime;
use super::{AzureMediaRequest, AzureMediaResult, edit, image, speech, transcription};

// The closed provider enum is the only entry: the typed call names the Azure leaf, and the resolved runtime carries its leaves.
pub async fn dispatch<C, K, T, L>(
    request: OperationRequest,
    operation: AzureMediaOperation,
    runtime: &mut MediaRuntime<'_, C, K, T, L>,
) -> Result<OperationResult, AzureMediaError>
where
    C: ImageClientConstructor,
    K: ImageClock,
    T: ImageTransport + TranscriptionTransport,
    L: ImageGenerationLogger + AzureTranscriptionLogger + Send + Sync,
{
    let OperationRequest::AzureMedia(AzureMediaRequest) = request else {
        return Err(AzureMediaError::UnsupportedOperation);
    };
    let result = match operation {
        AzureMediaOperation::ImageGeneration(call) => {
            AzureMediaResult::ImageGeneration(Box::new(image::generation(*call, runtime).await?))
        }
        AzureMediaOperation::ImageEdit(call) => {
            AzureMediaResult::ImageEdit(Box::new(edit::image_edit(*call, runtime).await?))
        }
        // B537's two speech leaves: the binary Azure OpenAI client and the native AVA dispatch through B2452
        AzureMediaOperation::OpenAiSpeech(call) => {
            AzureMediaResult::OpenAiSpeech(Box::new(speech::openai::openai(*call, runtime).await?))
        }
        AzureMediaOperation::NativeSsml(call) => {
            AzureMediaResult::NativeSsml(Box::new(speech::native::native(*call, runtime).await?))
        }
        AzureMediaOperation::Transcription(call) => {
            AzureMediaResult::Transcription(Box::new(transcription::openai(*call, runtime).await?))
        }
        AzureMediaOperation::NativeTranscription(call) => AzureMediaResult::NativeTranscription(
            Box::new(transcription::native(&call, runtime.transport).await?),
        ),
    };
    Ok(OperationResult::AzureMedia(result))
}
