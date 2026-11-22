use super::backend::{HealthProbeBackend, OcrHealthDocuments};
use super::batch::probe_batch_mode;
use super::calls::{
    AudioTranscriptionCall, HealthProbeReply, OcrCall, ProbeCall, RealtimeCall, add_parameter,
    ensure_absent,
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
// the modes whose backend method takes the probe call built from the health check parameters
enum ProbeCallMode {
    Chat,
    Completion,
    Embedding,
    AudioSpeech,
    ImageGeneration,
    VideoGeneration,
    Rerank,
    Responses,
}
// the modes whose handler builds a call of its own before the backend sends it
enum SpecialMode {
    AudioTranscription,
    ImageEdit,
    Realtime,
    Batch,
    Ocr,
    Evaluation,
}
// Ok for a mode that sends the probe call as it stands, Err for one with its own handler
const fn probe_route(mode: HealthProbeMode) -> Result<ProbeCallMode, SpecialMode> {
    match mode {
        HealthProbeMode::Chat => Ok(ProbeCallMode::Chat),
        HealthProbeMode::Completion => Ok(ProbeCallMode::Completion),
        HealthProbeMode::Embedding => Ok(ProbeCallMode::Embedding),
        HealthProbeMode::AudioSpeech => Ok(ProbeCallMode::AudioSpeech),
        HealthProbeMode::ImageGeneration => Ok(ProbeCallMode::ImageGeneration),
        HealthProbeMode::VideoGeneration => Ok(ProbeCallMode::VideoGeneration),
        HealthProbeMode::Rerank => Ok(ProbeCallMode::Rerank),
        HealthProbeMode::Responses => Ok(ProbeCallMode::Responses),
        HealthProbeMode::AudioTranscription => Err(SpecialMode::AudioTranscription),
        HealthProbeMode::ImageEdit => Err(SpecialMode::ImageEdit),
        HealthProbeMode::Realtime => Err(SpecialMode::Realtime),
        HealthProbeMode::Batch => Err(SpecialMode::Batch),
        HealthProbeMode::Ocr => Err(SpecialMode::Ocr),
        HealthProbeMode::Evaluation => Err(SpecialMode::Evaluation),
    }
}
async fn send_probe_call(
    mode: ProbeCallMode,
    backend: &dyn HealthProbeBackend,
    call: ProbeCall,
) -> Result<HealthProbeReply, HealthProbeError> {
    match mode {
        ProbeCallMode::Chat => backend.chat(call).await,
        ProbeCallMode::Completion => backend.completion(call).await,
        ProbeCallMode::Embedding => backend.embedding(call).await,
        ProbeCallMode::AudioSpeech => backend.audio_speech(call).await,
        ProbeCallMode::ImageGeneration => backend.image_generation(call).await,
        ProbeCallMode::VideoGeneration => backend.video_generation(call).await,
        ProbeCallMode::Rerank => backend.rerank(call).await,
        ProbeCallMode::Responses => backend.responses(call).await,
    }
}
pub async fn run_health_probe(
    plan: HealthProbePlan,
    backend: &dyn HealthProbeBackend,
    documents: &dyn OcrHealthDocuments,
) -> Result<HealthProbeReply, HealthProbeError> {
    let probe = ProbeRequest {
        model: &plan.model,
        custom_llm_provider: &plan.custom_llm_provider,
        params: &plan.params,
        prompt: plan.prompt.as_deref(),
        input: plan.input.as_deref(),
    };
    match probe_route(plan.mode) {
        Ok(mode) => {
            let call = ProbeCall {
                parameters: mode_parameters(plan.mode, &probe)?,
                api_key: probe.params.api_key.clone(),
                logging: probe.params.litellm_logging_obj.clone(),
            };
            send_probe_call(mode, backend, call).await
        }
        Err(SpecialMode::AudioTranscription) => {
            backend
                .audio_transcription(audio_transcription_call(plan.mode, &probe)?)
                .await
        }
        Err(SpecialMode::ImageEdit) => probe_image_edit(probe.params, backend).await,
        Err(SpecialMode::Realtime) => backend.realtime(realtime_call(&probe)?).await,
        Err(SpecialMode::Batch) => {
            probe_batch_mode(probe.custom_llm_provider, probe.params, backend).await
        }
        Err(SpecialMode::Ocr) => backend.ocr(ocr_call(plan.mode, &probe, documents)?).await,
        Err(SpecialMode::Evaluation) => {
            let call = evaluation_call(probe.params, probe.prompt, probe.custom_llm_provider)?;
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
        .call_parameters(mode == HealthProbeMode::Chat)?;
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
    let prompt = request.prompt;
    match mode {
        HealthProbeMode::Completion => {
            let text = prompt_or(prompt, "test").to_owned();
            vec![("prompt", Json::Text(text))]
        }
        HealthProbeMode::AudioSpeech | HealthProbeMode::Responses => {
            let text = prompt_or(prompt, "test").to_owned();
            vec![("input", Json::Text(text))]
        }
        HealthProbeMode::Embedding => vec![("input", Json::List(embedding_inputs(request.input)))],
        HealthProbeMode::ImageGeneration => {
            let value = prompt.map(String::from).map_or(Json::Null, Json::Text);
            vec![("prompt", value)]
        }
        HealthProbeMode::VideoGeneration => {
            let text = prompt_or(prompt, "test video generation").to_owned();
            vec![("prompt", Json::Text(text))]
        }
        HealthProbeMode::Rerank => {
            let query = Json::Text(prompt_or(prompt, "").to_owned());
            let documents = Json::List(vec![Json::Text("my sample text".to_owned())]);
            vec![("query", query), ("documents", documents)]
        }
        HealthProbeMode::Chat
        | HealthProbeMode::AudioTranscription
        | HealthProbeMode::ImageEdit
        | HealthProbeMode::Realtime
        | HealthProbeMode::Batch
        | HealthProbeMode::Ocr
        | HealthProbeMode::Evaluation => Vec::new(),
    }
}
fn embedding_inputs(input: Option<&[String]>) -> Vec<Json> {
    match input {
        Some(values) if !values.is_empty() => values.iter().cloned().map(Json::Text).collect(),
        None | Some(_) => vec![Json::Text("test".to_owned())],
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
fn realtime_call(request: &ProbeRequest<'_>) -> Result<RealtimeCall, HealthProbeError> {
    let call = RealtimeCall {
        model: request.model.to_owned(),
        custom_llm_provider: request.custom_llm_provider.to_owned(),
        api_base: request.params.api_base.as_option().cloned(),
        api_key: request.params.api_key.clone(),
        api_version: request.params.api_version.as_option().cloned(),
        model_params: request.params.call_parameters(true)?,
        logging: request.params.litellm_logging_obj.clone(),
    };
    Ok(call)
}
const fn prompt_or<'text>(prompt: Option<&'text str>, fallback: &'text str) -> &'text str {
    match prompt {
        Some(text) if !text.is_empty() => text,
        None | Some(_) => fallback,
    }
}
