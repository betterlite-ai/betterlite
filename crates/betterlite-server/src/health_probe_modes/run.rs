use super::backend::{HealthProbeBackend, OcrHealthDocuments};
use super::batch::probe_batch_mode;
use super::calls::{
    AudioTranscriptionCall, HealthProbeReply, OcrCall, RealtimeCall, add_parameter, ensure_absent,
    probe_call,
};
use super::error::HealthProbeError;
use super::evaluation::evaluation_call;
use super::image_edit::probe_image_edit;
use super::mode::HealthProbeMode;
use super::params::HealthProbeParams;
use super::plan::HealthProbePlan;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::HEALTH_CHECK_AUDIO;
struct ProbeRequest<'plan> {
    model: &'plan str,
    custom_llm_provider: &'plan str,
    params: &'plan HealthProbeParams,
    prompt: Option<&'plan str>,
    input: Option<&'plan [String]>,
}
pub async fn run_health_probe(
    plan: HealthProbePlan,
    backend: &dyn HealthProbeBackend,
    documents: &dyn OcrHealthDocuments,
) -> Result<HealthProbeReply, HealthProbeError> {
    let request = ProbeRequest {
        model: &plan.model,
        custom_llm_provider: &plan.custom_llm_provider,
        params: &plan.params,
        prompt: plan.prompt.as_deref(),
        input: plan.input.as_deref(),
    };
    match plan.mode {
        HealthProbeMode::Chat => {
            let call = probe_call(request.params.call_parameters(true), request.params);
            backend.chat(call).await
        }
        HealthProbeMode::Completion => run_completion(&request, backend).await,
        HealthProbeMode::Embedding => run_embedding(&request, backend).await,
        HealthProbeMode::AudioSpeech => run_audio_speech(&request, backend).await,
        HealthProbeMode::AudioTranscription => run_audio_transcription(&request, backend).await,
        HealthProbeMode::ImageGeneration => run_image_generation(&request, backend).await,
        HealthProbeMode::ImageEdit => probe_image_edit(request.params, backend).await,
        HealthProbeMode::VideoGeneration => run_video_generation(&request, backend).await,
        HealthProbeMode::Rerank => run_rerank(&request, backend).await,
        HealthProbeMode::Realtime => run_realtime(&request, backend).await,
        HealthProbeMode::Batch => {
            probe_batch_mode(request.custom_llm_provider, request.params, backend).await
        }
        HealthProbeMode::Responses => run_responses(&request, backend).await,
        HealthProbeMode::Ocr => run_ocr(&request, documents, backend).await,
        HealthProbeMode::Evaluation => {
            let call =
                evaluation_call(request.params, request.prompt, request.custom_llm_provider)?;
            backend.evaluation(call).await
        }
    }
}
fn prompt_or<'text>(prompt: Option<&'text str>, fallback: &'text str) -> &'text str {
    match prompt {
        Some(text) if !text.is_empty() => text,
        _ => fallback,
    }
}
async fn run_completion(
    request: &ProbeRequest<'_>,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let mut parameters = request.params.call_parameters(false);
    let prompt = prompt_or(request.prompt, "test");
    add_parameter(
        &mut parameters,
        "prompt",
        Json::Text(prompt.to_owned()),
        HealthProbeMode::Completion,
    )?;
    backend
        .completion(probe_call(parameters, request.params))
        .await
}
async fn run_embedding(
    request: &ProbeRequest<'_>,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let mut parameters = request.params.call_parameters(false);
    let inputs: Vec<Json> = match request.input {
        Some(values) if !values.is_empty() => values.iter().cloned().map(Json::Text).collect(),
        _ => vec![Json::Text("test".to_owned())],
    };
    add_parameter(
        &mut parameters,
        "input",
        Json::List(inputs),
        HealthProbeMode::Embedding,
    )?;
    backend
        .embedding(probe_call(parameters, request.params))
        .await
}
async fn run_audio_speech(
    request: &ProbeRequest<'_>,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let mut parameters = request.params.call_parameters(false);
    if !parameters.contains_key("voice") {
        parameters.insert("voice".to_owned(), Json::Text("alloy".to_owned()));
    }
    let prompt = prompt_or(request.prompt, "test");
    add_parameter(
        &mut parameters,
        "input",
        Json::Text(prompt.to_owned()),
        HealthProbeMode::AudioSpeech,
    )?;
    backend
        .audio_speech(probe_call(parameters, request.params))
        .await
}
async fn run_audio_transcription(
    request: &ProbeRequest<'_>,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let parameters = request.params.call_parameters(false);
    ensure_absent(&parameters, "file", HealthProbeMode::AudioTranscription)?;
    let call = AudioTranscriptionCall {
        parameters,
        file: HEALTH_CHECK_AUDIO.to_vec(),
        api_key: request.params.api_key.clone(),
        logging: request.params.litellm_logging_obj.clone(),
    };
    backend.audio_transcription(call).await
}
async fn run_image_generation(
    request: &ProbeRequest<'_>,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let mut parameters = request.params.call_parameters(false);
    let prompt = request
        .prompt
        .map_or(Json::Null, |text| Json::Text(text.to_owned()));
    add_parameter(
        &mut parameters,
        "prompt",
        prompt,
        HealthProbeMode::ImageGeneration,
    )?;
    backend
        .image_generation(probe_call(parameters, request.params))
        .await
}
async fn run_video_generation(
    request: &ProbeRequest<'_>,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let mut parameters = request.params.call_parameters(false);
    let prompt = prompt_or(request.prompt, "test video generation");
    add_parameter(
        &mut parameters,
        "prompt",
        Json::Text(prompt.to_owned()),
        HealthProbeMode::VideoGeneration,
    )?;
    backend
        .video_generation(probe_call(parameters, request.params))
        .await
}
async fn run_rerank(
    request: &ProbeRequest<'_>,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let mut parameters = request.params.call_parameters(false);
    let query = prompt_or(request.prompt, "");
    add_parameter(
        &mut parameters,
        "query",
        Json::Text(query.to_owned()),
        HealthProbeMode::Rerank,
    )?;
    add_parameter(
        &mut parameters,
        "documents",
        Json::List(vec![Json::Text("my sample text".to_owned())]),
        HealthProbeMode::Rerank,
    )?;
    backend.rerank(probe_call(parameters, request.params)).await
}
async fn run_realtime(
    request: &ProbeRequest<'_>,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let call = RealtimeCall {
        model: request.model.to_owned(),
        custom_llm_provider: request.custom_llm_provider.to_owned(),
        api_base: request.params.api_base.as_option().cloned(),
        api_key: request.params.api_key.clone(),
        api_version: request.params.api_version.as_option().cloned(),
        model_params: request.params.call_parameters(true),
        logging: request.params.litellm_logging_obj.clone(),
    };
    backend.realtime(call).await
}
async fn run_responses(
    request: &ProbeRequest<'_>,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let mut parameters = request.params.call_parameters(false);
    let input = prompt_or(request.prompt, "test");
    add_parameter(
        &mut parameters,
        "input",
        Json::Text(input.to_owned()),
        HealthProbeMode::Responses,
    )?;
    backend
        .responses(probe_call(parameters, request.params))
        .await
}
async fn run_ocr(
    request: &ProbeRequest<'_>,
    documents: &dyn OcrHealthDocuments,
    backend: &dyn HealthProbeBackend,
) -> Result<HealthProbeReply, HealthProbeError> {
    let parameters = request.params.call_parameters(false);
    ensure_absent(&parameters, "document", HealthProbeMode::Ocr)?;
    let document = documents.health_check_document(request.model, request.custom_llm_provider)?;
    let call = OcrCall {
        parameters,
        document,
        api_key: request.params.api_key.clone(),
        logging: request.params.litellm_logging_obj.clone(),
    };
    backend.ocr(call).await
}
