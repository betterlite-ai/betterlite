use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_models::catalog::ModelCatalog;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::gemini_live_setup::catalog::coerce_response_modalities;
use crate::gemini_live_setup::error::LiveSetupError;
use crate::gemini_live_setup::session::{SessionUpdate, normalize_session};
use crate::gemini_live_setup::setup::map_session;
use crate::gemini_live_setup::wire::{
    AutomaticActivityDetection, Envelope, LiveSetup, RealtimeInputConfig,
};

use super::error::VertexLiveError;
use super::policy::{DEFAULT_SILENCE_DURATION_MS, VertexLivePolicy};

pub struct VertexLiveInput<'input> {
    pub policy: &'input VertexLivePolicy,
    pub catalog: &'input ModelCatalog,
    pub schema: &'input SchemaSettings,
    pub model: &'input str,
    pub session_configuration_request: Option<&'input str>,
}

pub fn transform_session_update(
    session: &SessionUpdate,
    input: &VertexLiveInput<'_>,
) -> Result<Vec<String>, VertexLiveError> {
    if input.session_configuration_request.is_some() {
        log_dropped_update(session);
        return Ok(Vec::new());
    }
    let setup = vertex_setup(session, input)?;
    Ok(vec![
        serde_json::to_string(&Envelope { setup: &setup }).map_err(LiveSetupError::Encode)?,
    ])
}

fn vertex_setup(
    session: &SessionUpdate,
    input: &VertexLiveInput<'_>,
) -> Result<LiveSetup, VertexLiveError> {
    let mut setup = map_session(session, input.schema)?;
    setup.model = Some(input.policy.model_path(input.model));
    setup
        .generation_config
        .response_modalities
        .get_or_insert_with(|| vec!["AUDIO".to_owned()]);
    let auto_response_disabled = auto_response_disabled(session);
    let realtime_input = setup
        .realtime_input_config
        .get_or_insert_with(RealtimeInputConfig::default);
    let detection = realtime_input
        .automatic_activity_detection
        .get_or_insert_with(AutomaticActivityDetection::default);
    if !auto_response_disabled {
        // the guardrail sends a literal create_response=false to suppress the auto-response, so only that value keeps VAD off
        detection.disabled = Some(false);
    }
    detection
        .silence_duration_ms
        .get_or_insert_with(|| Json::Number(Number::from(DEFAULT_SILENCE_DURATION_MS)));
    setup
        .input_audio_transcription
        .get_or_insert_with(Object::new);
    setup
        .output_audio_transcription
        .get_or_insert_with(Object::new);
    if let Some(modalities) = setup.generation_config.response_modalities.as_deref() {
        let coerced = coerce_response_modalities(input.catalog, input.model, modalities);
        setup.generation_config.response_modalities = Some(coerced);
    }
    Ok(setup)
}

fn log_dropped_update(session: &SessionUpdate) {
    if auto_response_disabled(session) {
        tracing::warn!(
            "Vertex AI Realtime: dropping a later session.update that turns off automatic responses because Vertex Live rejects a second setup message, so the guardrail cannot suppress the model's auto-response"
        );
    } else {
        tracing::debug!("Vertex AI Realtime: ignoring a later session.update");
    }
}

fn auto_response_disabled(session: &SessionUpdate) -> bool {
    normalize_session(session)
        .turn_detection
        .and_then(|detection| detection.create_response)
        == Some(false)
}
