use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_models::catalog::ModelCatalog;
use betterlite_prompts::arguments::json::Object;

use super::catalog::{coerce_response_modalities, default_response_modality};
use super::error::LiveSetupError;
use super::session::{SessionUpdate, normalize_session};
use super::settings::LiveSetupSettings;
use super::turn_detection::map_automatic_turn_detection;
use super::voice::{SpeechSelection, speech_config};
use super::wire::{
    GenerationConfig, Instruction, InstructionPart, LiveSetup, RealtimeInputConfig,
    SystemInstruction, encode_envelope,
};
use crate::gemini_tools::assembly::map_tools;

pub fn map_session(
    session: &SessionUpdate,
    schema: &SchemaSettings,
) -> Result<LiveSetup, LiveSetupError> {
    let session = normalize_session(session);
    let mut setup = setup_from_session(&session);
    if let Some(tools) = &session.tools {
        // LiteLLM puts tools at the top level of the setup, outside generationConfig
        let mapped = map_tools(tools, false, None, schema)?;
        setup.tools = Some(mapped.tools);
        setup.tool_config = mapped.tool_config;
    }
    if let Some(turn_detection) = &session.turn_detection
        && let Some(detection) = map_automatic_turn_detection(turn_detection)
    {
        setup.realtime_input_config = Some(RealtimeInputConfig {
            automatic_activity_detection: Some(detection),
        });
    }
    if let Some(voice) = session.voice.as_deref()
        && let SpeechSelection::Voice(config) = speech_config(voice)?
    {
        setup.generation_config.speech_config = Some(config);
    }
    Ok(setup)
}

pub fn transform_session_update(
    session: &SessionUpdate,
    model: &str,
    session_configuration_request: Option<&str>,
    settings: &LiveSetupSettings<'_>,
) -> Result<Vec<String>, LiveSetupError> {
    let mut setup = map_session(session, settings.schema)?;
    if session_configuration_request.is_some() {
        log_dropped_update(session);
        return Ok(Vec::new());
    }
    fill_first_setup_defaults(&mut setup, model, settings.catalog);
    setup.model = Some(format!("models/{model}"));
    Ok(vec![encode_envelope(&setup)?])
}

fn setup_from_session(session: &SessionUpdate) -> LiveSetup {
    LiveSetup {
        model: None,
        generation_config: GenerationConfig {
            temperature: session.temperature.clone(),
            max_output_tokens: session.max_response_output_tokens.clone(),
            response_modalities: session.modalities.as_ref().map(|modalities| {
                modalities
                    .iter()
                    .map(|modality| modality.to_uppercase())
                    .collect()
            }),
            speech_config: None,
        },
        system_instruction: session.instructions.as_ref().map(|text| {
            Instruction::Content(SystemInstruction {
                role: "user".to_owned(),
                parts: vec![InstructionPart { text: text.clone() }],
            })
        }),
        tools: None,
        tool_config: None,
        realtime_input_config: None,
        session_resumption: None,
        session_resumption_config: None,
        context_window_compression: None,
        input_audio_transcription: session
            .input_audio_transcription
            .is_some()
            .then(Object::new),
        output_audio_transcription: None,
    }
}

fn fill_first_setup_defaults(setup: &mut LiveSetup, model: &str, catalog: &ModelCatalog) {
    if setup.generation_config.response_modalities.is_none() {
        setup.generation_config.response_modalities =
            Some(vec![default_response_modality(catalog, model).to_owned()]);
    }
    setup
        .input_audio_transcription
        .get_or_insert_with(Object::new);
    if let Some(modalities) = setup.generation_config.response_modalities.as_deref() {
        let coerced = coerce_response_modalities(catalog, model, modalities);
        setup.generation_config.response_modalities = Some(coerced);
    }
}

fn log_dropped_update(session: &SessionUpdate) {
    let normalized = normalize_session(session);
    let guardrail = normalized
        .turn_detection
        .as_ref()
        .and_then(|detection| detection.create_response)
        == Some(false);
    if guardrail {
        tracing::warn!(
            "Gemini Realtime: dropping the subsequent session.update with turn_detection.create_response=false because Gemini Live rejects a second setup message, so audio-transcription guardrails cannot suppress the model's auto-response mid-session"
        );
    } else {
        tracing::debug!(
            "Gemini Realtime: ignoring session.update because the setup was already sent"
        );
    }
}
