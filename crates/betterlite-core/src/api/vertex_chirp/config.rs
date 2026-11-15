use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;

use crate::api::api_boundaries::ApiLimits;
use crate::api::realtime_contracts::config::{BaseRealtimeConfig, Environment};
use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::realtime_contracts::events::{
    AudioTranscriptionUsage, RealtimeResponseTransformInput, RealtimeResponseTypedDict,
};
use crate::api::realtime_contracts::transport::RealtimeMessage;
use crate::api::transcription_protocol::audio::decode_pcm16_append;
use crate::api::transcription_protocol::events::{
    EventIdSource as _,
    // ast-grep-ignore: no-long-names
    transcription_session_created_event as created_event,
};
use crate::api::transcription_protocol::{JsonObject, json_object};
use crate::api::vertex_chirp_protocol::transformer::ChirpEventTransformer;
use crate::api::vertex_chirp_protocol::{
    ChirpSessionConfig, DISCARD_TURN_COMMAND, FINISH_TURN_COMMAND, MAX_AUDIO_MESSAGE_BYTES,
    default_session_config, is_vertex_speech_to_text_model, parse_chirp_session_update,
};

use super::decode::decode_chirp_backend_event;
use super::error::ChirpAdapterError;
use super::factory::ChirpBackendFactory;
use super::ids::{ChirpIds, SharedIds};
use super::location::{validate_location, validate_project};
use super::target::{AccessTokenResolver, SpeechStreamingTarget};
use super::url::{api_endpoint, speech_to_text_host};
use super::usage::audio_transcription_usage;

const CONFIGURE_EVENT_TYPES: [&str; 2] = ["session.update", "transcription_session.update"];
const APPEND_EVENT_TYPE: &str = "input_audio_buffer.append";
const COMMIT_EVENT_TYPES: [&str; 2] = ["input_audio_buffer.commit", "input_audio_buffer.end"];
const CLEAR_EVENT_TYPE: &str = "input_audio_buffer.clear";

pub struct ChirpSettings {
    pub project: String,
    pub location: Option<String>,
    pub catalog: Arc<ModelCatalog>,
    pub limits: ApiLimits,
    pub ids: Box<dyn ChirpIds>,
}

pub struct VertexChirpRealtimeConfig<Factory> {
    project: String,
    location: String,
    catalog: Arc<ModelCatalog>,
    limits: ApiLimits,
    token: Arc<dyn AccessTokenResolver>,
    factory: Factory,
    ids: SharedIds,
    state: Mutex<AdapterState>,
}

struct AdapterState {
    transformer: ChirpEventTransformer<SharedIds, SharedIds>,
    config: Option<ChirpSessionConfig>,
    session_id: Option<String>,
}

impl<Factory> VertexChirpRealtimeConfig<Factory> {
    pub fn new(
        settings: ChirpSettings,
        token: Arc<dyn AccessTokenResolver>,
        factory: Factory,
    ) -> Result<Self, ChirpAdapterError> {
        let ids = SharedIds(Arc::new(Mutex::new(settings.ids)));
        Ok(Self {
            project: validate_project(&settings.project)?,
            location: validate_location(settings.location.as_deref())?,
            catalog: settings.catalog,
            limits: settings.limits,
            token,
            factory,
            ids: ids.clone(),
            state: Mutex::new(AdapterState {
                transformer: ChirpEventTransformer::new(ids.clone(), ids),
                config: None,
                session_id: None,
            }),
        })
    }

    pub fn complete_url(
        &self,
        api_base: Option<&str>,
        model: &str,
    ) -> Result<String, ChirpAdapterError> {
        if !is_vertex_speech_to_text_model(&self.catalog, model) {
            return Err(ChirpAdapterError::UnsupportedModel {
                model: model.to_owned(),
            });
        }
        Ok(api_base.map_or_else(|| speech_to_text_host(&self.location), api_endpoint))
    }

    fn configure(
        &self,
        message: &str,
        model: &str,
    ) -> Result<Vec<RealtimeMessage>, ChirpAdapterError> {
        let config = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.config.is_some() {
                tracing::debug!(
                    "Speech-to-Text streaming: ignoring session.update after the stream was configured"
                );
                return Ok(Vec::new());
            }
            let session_config =
                parse_chirp_session_update(message.as_bytes(), model, self.limits)?;
            let session_id = state
                .session_id
                .clone()
                .unwrap_or_else(|| self.ids.session_id());
            state
                .transformer
                .configure(session_config.clone(), &session_id);
            state.config = Some(session_config.clone());
            session_config
        };
        let command = config.configure_command()?;
        Ok(vec![RealtimeMessage::Text(command)])
    }

    fn require_config(&self) -> Result<(), ChirpAdapterError> {
        if self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .config
            .is_none()
        {
            return Err(ChirpAdapterError::UnconfiguredAudio);
        }
        Ok(())
    }

    fn append_audio(
        &self,
        request: &JsonObject,
    ) -> Result<Vec<RealtimeMessage>, ChirpAdapterError> {
        self.require_config()?;
        let audio = decode_pcm16_append::<ChirpAdapterError>(request.get("audio"), None)?;
        Ok(audio
            .chunks(MAX_AUDIO_MESSAGE_BYTES)
            .map(<[u8]>::to_vec)
            .map(RealtimeMessage::Binary)
            .collect())
    }

    fn turn_command(&self, command: &str) -> Result<Vec<RealtimeMessage>, ChirpAdapterError> {
        self.require_config()?;
        Ok(vec![RealtimeMessage::Text(command.to_owned())])
    }
}

fn event_type(request: &JsonObject) -> Option<&str> {
    match request.get("type") {
        Some(Json::Text(text)) => Some(text.as_str()),
        _ => None,
    }
}

#[async_trait]
impl<Factory> BaseRealtimeConfig for VertexChirpRealtimeConfig<Factory>
where
    Factory: ChirpBackendFactory,
{
    type Error = ChirpAdapterError;
    type Backend = Factory::Backend;

    fn validate_environment(
        &self,
        environment: &Environment<'_>,
    ) -> Result<HeaderMap, Self::Error> {
        Ok(environment.headers.clone())
    }

    // the frozen contract returns a URL, so a rejection it cannot carry becomes an empty one
    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        model: &str,
        _api_key: Option<&str>,
    ) -> String {
        self.complete_url(api_base, model).unwrap_or_default()
    }

    fn transform_realtime_request(
        &self,
        message: &str,
        model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, Self::Error> {
        let request = json_object::<ChirpAdapterError>(message.as_bytes())?;
        match event_type(&request) {
            Some(kind) if CONFIGURE_EVENT_TYPES.contains(&kind) => self.configure(message, model),
            Some(APPEND_EVENT_TYPE) => self.append_audio(&request),
            Some(kind) if COMMIT_EVENT_TYPES.contains(&kind) => {
                self.turn_command(FINISH_TURN_COMMAND)
            }
            Some(CLEAR_EVENT_TYPE) => self.turn_command(DISCARD_TURN_COMMAND),
            _ => {
                tracing::debug!(
                    "Speech-to-Text streaming: dropping unsupported client event {:?}",
                    request.get("type")
                );
                Ok(Vec::new())
            }
        }
    }

    fn is_setup_message(&self, message: &Json) -> bool {
        let Json::Object(fields) = message else {
            return false;
        };
        fields
            .get("kind")
            .is_some_and(|kind| matches!(kind, Json::Text(text) if text == "configure"))
    }

    async fn open_backend(
        &self,
        url: &str,
        _headers: &HeaderMap,
    ) -> Result<Option<Self::Backend>, Self::Error> {
        Ok(Some(self.factory.open(SpeechStreamingTarget {
            api_endpoint: url.to_owned(),
            recognizer: format!(
                "projects/{}/locations/{}/recognizers/_",
                self.project, self.location
            ),
            token: Arc::clone(&self.token),
        })))
    }

    fn session_created_event(
        &self,
        model: &str,
        logging_session_id: &str,
        _session_configuration_request: Option<&str>,
    ) -> Option<Json> {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.session_id = Some(logging_session_id.to_owned());
        }
        let session = default_session_config(model).openai_session(logging_session_id);
        let event = created_event(self.ids.clone().next_event_id(), &session);
        serde_json::to_value(&event)
            .and_then(serde_json::from_value::<Json>)
            .ok()
    }

    fn session_close_usage(&self, _model: &str) -> Option<AudioTranscriptionUsage> {
        let usage = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.transformer.take_unbilled_usage().ok()?
        };
        usage.and_then(audio_transcription_usage)
    }

    fn transform_realtime_response(
        &self,
        message: &RealtimeMessage,
        _model: &str,
        input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, Self::Error> {
        let bytes = match message {
            RealtimeMessage::Text(text) => text.as_bytes(),
            RealtimeMessage::Binary(bytes) => bytes.as_slice(),
        };
        let frame = decode_chirp_backend_event(bytes, self.limits)?;
        let events = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.transformer.transform(frame)?
        };
        Ok(RealtimeResponseTypedDict {
            response: serde_json::to_value(&events)
                .and_then(serde_json::from_value::<Json>)
                .map_err(|source| ChirpAdapterError::Encode { source })?,
            current_output_item_id: input.current_output_item_id.clone(),
            current_response_id: input.current_response_id.clone(),
            current_delta_chunks: input.current_delta_chunks.clone(),
            current_conversation_id: input.current_conversation_id.clone(),
            current_item_chunks: input.current_item_chunks.clone(),
            current_delta_type: input.current_delta_type,
            session_configuration_request: input.session_configuration_request.clone(),
        })
    }
}
