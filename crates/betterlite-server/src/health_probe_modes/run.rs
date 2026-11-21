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
use indexmap::IndexMap;
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
    let mode = plan.mode;
    let request = ProbeRequest {
        model: &plan.model,
        custom_llm_provider: &plan.custom_llm_provider,
        params: &plan.params,
        prompt: plan.prompt.as_deref(),
        input: plan.input.as_deref(),
    };
    match mode {
        HealthProbeMode::Chat => {
            backend
                .chat(probe_call(mode_parameters(mode, &request)?, request.params))
                .await
        }
        HealthProbeMode::Completion => {
            backend
                .completion(probe_call(mode_parameters(mode, &request)?, request.params))
                .await
        }
        HealthProbeMode::Embedding => {
            backend
                .embedding(probe_call(mode_parameters(mode, &request)?, request.params))
                .await
        }
        HealthProbeMode::AudioSpeech => {
            backend
                .audio_speech(probe_call(mode_parameters(mode, &request)?, request.params))
                .await
        }
        HealthProbeMode::ImageGeneration => {
            backend
                .image_generation(probe_call(mode_parameters(mode, &request)?, request.params))
                .await
        }
        HealthProbeMode::VideoGeneration => {
            backend
                .video_generation(probe_call(mode_parameters(mode, &request)?, request.params))
                .await
        }
        HealthProbeMode::Rerank => {
            backend
                .rerank(probe_call(mode_parameters(mode, &request)?, request.params))
                .await
        }
        HealthProbeMode::Responses => {
            backend
                .responses(probe_call(mode_parameters(mode, &request)?, request.params))
                .await
        }
        HealthProbeMode::AudioTranscription => {
            let call = audio_transcription_call(mode, &request)?;
            backend.audio_transcription(call).await
        }
        HealthProbeMode::ImageEdit => probe_image_edit(request.params, backend).await,
        HealthProbeMode::Realtime => backend.realtime(realtime_call(&request)).await,
        HealthProbeMode::Batch => {
            probe_batch_mode(request.custom_llm_provider, request.params, backend).await
        }
        HealthProbeMode::Ocr => {
            let call = ocr_call(mode, &request, documents)?;
            backend.ocr(call).await
        }
        HealthProbeMode::Evaluation => {
            let call =
                evaluation_call(request.params, request.prompt, request.custom_llm_provider)?;
            backend.evaluation(call).await
        }
    }
}
fn mode_parameters(
    mode: HealthProbeMode,
    request: &ProbeRequest<'_>,
) -> Result<IndexMap<String, Json>, HealthProbeError> {
    let mut parameters = request
        .params
        .call_parameters(mode == HealthProbeMode::Chat);
    if mode == HealthProbeMode::AudioSpeech && !parameters.contains_key("voice") {
        parameters.insert("voice".to_owned(), Json::Text("alloy".to_owned()));
    }
    for (key, value) in added_parameters(mode, request) {
        add_parameter(&mut parameters, key, value, mode)?;
    }
    Ok(parameters)
}
fn added_parameters(
    mode: HealthProbeMode,
    request: &ProbeRequest<'_>,
) -> Vec<(&'static str, Json)> {
    match mode {
        HealthProbeMode::Completion => vec![(
            "prompt",
            Json::Text(prompt_or(request.prompt, "test").to_owned()),
        )],
        HealthProbeMode::AudioSpeech | HealthProbeMode::Responses => vec![(
            "input",
            Json::Text(prompt_or(request.prompt, "test").to_owned()),
        )],
        HealthProbeMode::Embedding => {
            let inputs = match request.input {
                Some(values) if !values.is_empty() => {
                    values.iter().cloned().map(Json::Text).collect()
                }
                None | Some(_) => vec![Json::Text("test".to_owned())],
            };
            vec![("input", Json::List(inputs))]
        }
        HealthProbeMode::ImageGeneration => vec![(
            "prompt",
            request
                .prompt
                .map_or(Json::Null, |text| Json::Text(text.to_owned())),
        )],
        HealthProbeMode::VideoGeneration => vec![(
            "prompt",
            Json::Text(prompt_or(request.prompt, "test video generation").to_owned()),
        )],
        HealthProbeMode::Rerank => vec![
            (
                "query",
                Json::Text(prompt_or(request.prompt, "").to_owned()),
            ),
            (
                "documents",
                Json::List(vec![Json::Text("my sample text".to_owned())]),
            ),
        ],
        HealthProbeMode::Chat
        | HealthProbeMode::AudioTranscription
        | HealthProbeMode::ImageEdit
        | HealthProbeMode::Realtime
        | HealthProbeMode::Batch
        | HealthProbeMode::Ocr
        | HealthProbeMode::Evaluation => Vec::new(),
    }
}
fn audio_transcription_call(
    mode: HealthProbeMode,
    request: &ProbeRequest<'_>,
) -> Result<AudioTranscriptionCall, HealthProbeError> {
    let parameters = mode_parameters(mode, request)?;
    ensure_absent(&parameters, "file", mode)?;
    let call = AudioTranscriptionCall {
        parameters,
        file: HEALTH_CHECK_AUDIO.to_vec(),
        api_key: request.params.api_key.clone(),
        logging: request.params.litellm_logging_obj.clone(),
    };
    Ok(call)
}
fn realtime_call(request: &ProbeRequest<'_>) -> RealtimeCall {
    RealtimeCall {
        model: request.model.to_owned(),
        custom_llm_provider: request.custom_llm_provider.to_owned(),
        api_base: request.params.api_base.as_option().cloned(),
        api_key: request.params.api_key.clone(),
        api_version: request.params.api_version.as_option().cloned(),
        model_params: request.params.call_parameters(true),
        logging: request.params.litellm_logging_obj.clone(),
    }
}
fn ocr_call(
    mode: HealthProbeMode,
    request: &ProbeRequest<'_>,
    documents: &dyn OcrHealthDocuments,
) -> Result<OcrCall, HealthProbeError> {
    let parameters = mode_parameters(mode, request)?;
    ensure_absent(&parameters, "document", mode)?;
    let document = documents.health_check_document(request.model, request.custom_llm_provider)?;
    let call = OcrCall {
        parameters,
        document,
        api_key: request.params.api_key.clone(),
        logging: request.params.litellm_logging_obj.clone(),
    };
    Ok(call)
}
fn prompt_or<'text>(prompt: Option<&'text str>, fallback: &'text str) -> &'text str {
    match prompt {
        Some(text) if !text.is_empty() => text,
        None | Some(_) => fallback,
    }
}
