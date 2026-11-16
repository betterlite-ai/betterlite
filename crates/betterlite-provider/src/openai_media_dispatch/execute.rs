use betterlite_openai::image_edits::environment::complete_url;
use betterlite_openai::image_edits::family::select_image_edit_family;
use betterlite_openai::image_edits::file::multipart_parts;
use betterlite_openai::image_edits::request::transform_image_edit_request;
use betterlite_openai::image_generation::call::ImageGenerationCall;
use betterlite_openai::image_generation::logging::ImageGenerationLogger;
use betterlite_openai::image_generation::request::{
    ImageGenerationOutcome, begin_image_generation, finish_image_generation, send_image_generation,
};
use betterlite_openai::image_generation::transport::ImageGenerationTransport;
use betterlite_openai::image_variations::ImageVariationRequest;
use betterlite_openai::image_variations::OpenAiImageVariationConfig;
use betterlite_openai::image_variations::logging::ImageVariationLogger;
use betterlite_openai::image_variations::request::{
    ImageVariationOutcome, begin_image_variation, finish_image_variation, send_image_variation,
};
use betterlite_openai::image_variations::transport::ImageVariationTransport;
use betterlite_openai::speech::call::{SpeechCall, audio_speech};
use betterlite_openai::speech::logging::SpeechLogger;
use betterlite_openai::speech::request::SpeechRequest;
use betterlite_openai::speech::result::{SpeechResponse, SpeechTransport};
use betterlite_openai::transcription::call::{
    TranscriptionCall, TranscriptionRequest, audio_transcriptions, finish_transcription,
};
use betterlite_openai::transcription::logging::TranscriptionLogger;
use betterlite_openai::transcription::reply::TranscriptionReply;
use betterlite_openai::transcription::transport::TranscriptionTransport;
use betterlite_openai::videos::VideoOptions;

use crate::operation::{OperationRequest, OperationResult};

use super::call::{ImageEditCall, OpenAiMediaOperation, SpeechOperation, TranscriptionOperation};
use super::error::OpenAiMediaError;
use super::parts::http_part;
use super::{
    MediaMethod, MediaPayload, OpenAiMediaPrepared, OpenAiMediaRequest, OpenAiMediaResult,
};

// The closed provider enum is the only entry: every other operation stays outside this composition, and the typed call names the family.
pub async fn dispatch<T, L>(
    request: OperationRequest,
    operation: OpenAiMediaOperation,
    transport: &T,
    logger: &mut L,
) -> Result<OperationResult, OpenAiMediaError>
where
    T: ImageGenerationTransport
        + ImageVariationTransport
        + SpeechTransport
        + TranscriptionTransport,
    L: ImageGenerationLogger + ImageVariationLogger + SpeechLogger + TranscriptionLogger,
{
    let OperationRequest::OpenAiMedia(OpenAiMediaRequest) = request else {
        return Err(OpenAiMediaError::UnsupportedOperation);
    };
    let result = match operation {
        OpenAiMediaOperation::ImageGeneration(call) => OpenAiMediaResult::ImageGeneration(
            Box::new(image_generation(*call, transport, logger).await?),
        ),
        OpenAiMediaOperation::ImageEdit(call) => {
            OpenAiMediaResult::ImageEdit(Box::new(image_edit(&call)?))
        }
        OpenAiMediaOperation::ImageVariation(variation) => OpenAiMediaResult::ImageVariation(
            Box::new(image_variation(*variation, transport, logger).await?),
        ),
        OpenAiMediaOperation::Speech(operation) => {
            OpenAiMediaResult::Speech(Box::new(speech(*operation, transport, logger).await?))
        }
        OpenAiMediaOperation::Transcription(operation) => OpenAiMediaResult::Transcription(
            Box::new(transcription(*operation, transport, logger).await?),
        ),
        OpenAiMediaOperation::Video(operation) => {
            OpenAiMediaResult::Video(Box::new(super::video::video(*operation)?))
        }
        OpenAiMediaOperation::VideoEdit(operation) => {
            OpenAiMediaResult::VideoEdit(Box::new(super::video_edit::video_edit(*operation)?))
        }
    };
    Ok(OperationResult::OpenAiMedia(result))
}

async fn image_generation<T, L>(
    call: ImageGenerationCall,
    transport: &T,
    logger: &mut L,
) -> Result<ImageGenerationOutcome, OpenAiMediaError>
where
    T: ImageGenerationTransport,
    L: ImageGenerationLogger,
{
    let prepared = begin_image_generation(call, logger)?;
    let sent = send_image_generation(&prepared, transport).await;
    Ok(finish_image_generation(&prepared, sent, logger)?)
}

async fn image_variation<T, L>(
    request: ImageVariationRequest,
    transport: &T,
    logger: &mut L,
) -> Result<ImageVariationOutcome, OpenAiMediaError>
where
    T: ImageVariationTransport,
    L: ImageVariationLogger,
{
    // the dispatcher selects the OpenAI variation config that LiteLLM's provider manager would install
    let configured = ImageVariationRequest {
        config: Some(OpenAiImageVariationConfig),
        ..request
    };
    let prepared = begin_image_variation(configured, logger)?;
    let sent = send_image_variation(&prepared, transport).await;
    Ok(finish_image_variation(&prepared, sent, logger)?)
}

fn image_edit(call: &ImageEditCall) -> Result<OpenAiMediaPrepared, OpenAiMediaError> {
    let family = select_image_edit_family(&call.request.model);
    let form = transform_image_edit_request(family, &call.request)?;
    let url = complete_url(&call.api_base)?;
    Ok(OpenAiMediaPrepared {
        method: MediaMethod::Post,
        url: url.to_string(),
        query: VideoOptions::new(),
        body: MediaPayload::ImageData(form.data),
        parts: multipart_parts(&form.files)
            .into_iter()
            .map(http_part)
            .collect(),
        multipart: true,
    })
}

async fn speech<T, L>(
    operation: SpeechOperation,
    transport: &T,
    logger: &mut L,
) -> Result<SpeechResponse, OpenAiMediaError>
where
    T: SpeechTransport,
    L: SpeechLogger,
{
    let SpeechOperation {
        model,
        input,
        voice,
        options,
        connection,
        aspeech,
    } = operation;
    let call = SpeechCall {
        request: SpeechRequest {
            model: &model,
            input: &input,
            voice: &voice,
            options: &options,
        },
        connection,
        aspeech,
    };
    Ok(audio_speech(call, logger, transport)?.await?)
}

async fn transcription<T, L>(
    operation: TranscriptionOperation,
    transport: &T,
    logger: &mut L,
) -> Result<TranscriptionReply, OpenAiMediaError>
where
    T: TranscriptionTransport,
    L: TranscriptionLogger,
{
    let TranscriptionOperation {
        input,
        file,
        connection,
        boundary,
        model,
        family,
        optional_params,
        atranscription,
    } = operation;
    let call = TranscriptionCall {
        input,
        file: &file,
        connection,
        boundary,
    };
    let request = TranscriptionRequest {
        model: &model,
        family,
        optional_params,
        atranscription,
    };
    let (outbound, pending) = audio_transcriptions(&call, &request, logger)?;
    let sent = transport.send(outbound).await;
    Ok(finish_transcription(pending, sent, logger)?)
}
