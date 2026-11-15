mod tests;

use std::mem::take;
use std::ops::Add;
use std::ops::Div;
use std::ops::Sub;
use std::str::from_utf8;
use std::sync::Arc;

use betterlite_models::decisions::Json;
use futures_util::future::BoxFuture;
use secrecy::SecretString;
use tracing::debug;

use super::END_STREAM;
use super::MuseMode;
use super::MuseProtocolError;
use super::MuseSessionConfig;
use super::events::MuseEvent;
use super::events::MuseEventTransformer;
use super::events::event_json;
use super::events::text_field;
use super::session::normalize_access_token;
use super::session::parse_session_update;
use crate::api::api_boundaries::ApiLimits;
use crate::api::realtime_contracts::error::HeaderMap;
use crate::api::realtime_contracts::events::RealtimeResponseTransformInput;
use crate::api::realtime_contracts::events::RealtimeResponseTypedDict;
use crate::api::realtime_contracts::transport::RealtimeMessage;
use crate::api::transcription_protocol::JsonObject;
use crate::api::transcription_protocol::audio::decode_pcm16_append;
use crate::api::transcription_protocol::events::EventIdSource;
use crate::api::transcription_protocol::events::UuidEventIds;
// ast-grep-ignore: no-long-names
use crate::api::transcription_protocol::events::transcription_session_created_event as created_event;
use crate::api::transcription_protocol::json_object;
use crate::api::transcription_protocol::usage::TranscriptionUsage;

pub type Monotonic = Arc<dyn Fn() -> f64 + Send + Sync>;
pub type Sleep = Arc<dyn Fn(f64) -> BoxFuture<'static, ()> + Send + Sync>;

pub struct MetaRealtimeConfig<Ids = UuidEventIds> {
    monotonic: Monotonic,
    sleep: Sleep,
    limits: ApiLimits,
    ids: Ids,
    transformer: MuseEventTransformer,
    access_token: Option<SecretString>,
    session: Option<MuseSessionConfig>,
    pending_audio: Vec<u8>,
    end_stream_sent: bool,
    pacing_origin: Option<f64>,
    sent_duration: f64,
}

impl<Ids> MetaRealtimeConfig<Ids>
where
    Ids: EventIdSource,
{
    #[must_use]
    pub fn new(monotonic: Monotonic, sleep: Sleep, limits: ApiLimits, ids: Ids) -> Self {
        Self {
            monotonic,
            sleep,
            limits,
            ids,
            transformer: MuseEventTransformer::default(),
            access_token: None,
            session: None,
            pending_audio: Vec::new(),
            end_stream_sent: false,
            pacing_origin: None,
            sent_duration: 0.0,
        }
    }

    pub fn validate_environment(
        &mut self,
        headers: HeaderMap,
        _model: &str,
        api_key: Option<&str>,
        env_api_key: Option<&str>,
    ) -> Result<HeaderMap, MuseProtocolError> {
        let Some(configured) = api_key.or(env_api_key) else {
            return Err(MuseProtocolError::ApiKeyMissing);
        };
        let token = SecretString::from(configured.to_owned());
        self.access_token = Some(normalize_access_token(&token)?);
        Ok(headers)
    }

    #[must_use]
    // ast-grep-ignore: no-long-names
    pub fn transform_session_created_event(&mut self, logging_session_id: &str) -> MuseEvent {
        let session = MuseSessionConfig::default().openai_session(logging_session_id);
        let event_id = self.ids.next_event_id();
        MuseEvent::Created(created_event(event_id, &session))
    }

    pub fn transform_realtime_request(
        &mut self,
        message: &str,
        model: &str,
        _session_configuration_request: Option<&str>,
    ) -> Result<Vec<RealtimeMessage>, MuseProtocolError> {
        let request = json_object::<MuseProtocolError>(message.as_bytes())?;
        match text_field(request.get("type")) {
            Some("session.update" | "transcription_session.update") => {
                self.configure(message, model)
            }
            Some("input_audio_buffer.append") => self.append_audio(&request),
            Some("input_audio_buffer.commit") => {
                let Some(config) = self.session.as_ref() else {
                    return Err(MuseProtocolError::SessionNotConfigured);
                };
                let end_stream = config.mode == MuseMode::PushToTalk;
                Ok(self.flush_audio(end_stream))
            }
            Some("input_audio_buffer.end") => Ok(self.flush_audio(true)),
            Some("input_audio_buffer.clear") => {
                self.pending_audio.clear();
                Ok(Vec::new())
            }
            Some(other) => {
                debug!("Meta realtime: dropping unsupported client event {other}");
                Ok(Vec::new())
            }
            None => {
                debug!("Meta realtime: dropping unsupported client event");
                Ok(Vec::new())
            }
        }
    }

    pub async fn pace_backend_send(&mut self, message: &[u8]) -> Result<(), MuseProtocolError> {
        let now = (self.monotonic)();
        let effective_origin = match self.pacing_origin {
            Some(origin) if Add::add(origin, self.sent_duration) >= now => origin,
            Some(_) | None => Sub::sub(now, self.sent_duration),
        };
        let delay = Sub::sub(Add::add(effective_origin, self.sent_duration), now);
        if delay > 0.0_f64 {
            (self.sleep)(delay).await;
        }
        self.pacing_origin = Some(effective_origin);
        let Some(config) = self.session.as_ref() else {
            return Err(MuseProtocolError::SessionNotConfigured);
        };
        let bytes_per_second = config.sample_rate.bytes_per_second();
        let length =
            u32::try_from(message.len()).map_err(|_error| MuseProtocolError::MessageTooLarge)?;
        let packet_seconds = Div::div(f64::from(length), f64::from(bytes_per_second));
        self.sent_duration = Add::add(self.sent_duration, packet_seconds);
        Ok(())
    }

    #[must_use]
    // ast-grep-ignore: no-long-names, no-one-line-functions
    pub fn unbilled_usage_on_session_close(&mut self, _model: &str) -> Option<TranscriptionUsage> {
        self.transformer.take_unbilled_usage()
    }

    pub fn transform_realtime_response(
        &mut self,
        message: &RealtimeMessage,
        _model: &str,
        input: &RealtimeResponseTransformInput,
    ) -> Result<RealtimeResponseTypedDict, MuseProtocolError> {
        let payload = text_of(message)?;
        let response = self.response_events(payload)?;
        Ok(RealtimeResponseTypedDict {
            response,
            current_output_item_id: input.current_output_item_id.clone(),
            current_response_id: input.current_response_id.clone(),
            current_delta_chunks: input.current_delta_chunks.clone(),
            current_conversation_id: input.current_conversation_id.clone(),
            current_item_chunks: input.current_item_chunks.clone(),
            current_delta_type: input.current_delta_type,
            session_configuration_request: input.session_configuration_request.clone(),
        })
    }

    fn configure(
        &mut self,
        message: &str,
        model: &str,
    ) -> Result<Vec<RealtimeMessage>, MuseProtocolError> {
        if self.session.is_some() {
            debug!("Meta realtime: ignoring session.update after the Muse handshake was sent");
            return Ok(Vec::new());
        }
        let Some(access_token) = self.access_token.as_ref() else {
            return Err(MuseProtocolError::TokenNotValidated);
        };
        let config = parse_session_update(message, model, self.limits)?;
        let handshake = config.handshake(access_token);
        let text = serde_json::to_string(&handshake)
            .map_err(|source| MuseProtocolError::Encode { source })?;
        self.session = Some(config);
        Ok(vec![RealtimeMessage::Text(text)])
    }

    fn append_audio(
        &mut self,
        request: &JsonObject,
    ) -> Result<Vec<RealtimeMessage>, MuseProtocolError> {
        let Some(config) = self.session.as_ref() else {
            return Err(MuseProtocolError::SessionNotConfigured);
        };
        let packet_size = config.sample_rate.packet_bytes();
        let max_encoded_bytes = config.sample_rate.max_encoded_append_bytes();
        let audio = decode_pcm16_append::<MuseProtocolError>(
            request.get("audio"),
            Some(max_encoded_bytes),
        )?;
        let mut buffered = take(&mut self.pending_audio);
        buffered.extend_from_slice(&audio);
        let Some(remainder) = buffered.len().checked_rem(packet_size) else {
            return Err(MuseProtocolError::InvalidPacketSize);
        };
        let packet_end = buffered.len().saturating_sub(remainder);
        self.pending_audio = buffered.split_off(packet_end);
        let frames = buffered
            .chunks(packet_size)
            .map(|packet| RealtimeMessage::Binary(packet.to_vec()))
            .collect();
        Ok(frames)
    }

    fn flush_audio(&mut self, end_stream: bool) -> Vec<RealtimeMessage> {
        let remainder = take(&mut self.pending_audio);
        let mut frames = Vec::new();
        if !remainder.is_empty() {
            frames.push(RealtimeMessage::Binary(remainder));
        }
        if !end_stream || self.end_stream_sent {
            return frames;
        }
        self.end_stream_sent = true;
        frames.push(RealtimeMessage::Text(END_STREAM.to_owned()));
        frames
    }

    fn response_events(&mut self, payload: &str) -> Result<Json, MuseProtocolError> {
        let frame = json_object::<MuseProtocolError>(payload.as_bytes())?;
        let session_id = match frame.get("sessionId") {
            None | Some(Json::Null) => return self.transformer_events(&frame),
            Some(Json::Text(text)) => text.trim(),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(MuseProtocolError::InvalidHandshake);
            }
        };
        if session_id.is_empty() {
            return Err(MuseProtocolError::InvalidHandshake);
        }
        let Some(config) = self.session.as_ref() else {
            return Err(MuseProtocolError::SessionNotConfigured);
        };
        let event_id = self.ids.next_event_id();
        let session = config.openai_session(session_id);
        event_json(&MuseEvent::Created(created_event(event_id, &session)))
    }

    fn transformer_events(&mut self, frame: &JsonObject) -> Result<Json, MuseProtocolError> {
        let mode = self
            .session
            .as_ref()
            .map_or(MuseMode::Endpointing, |config| config.mode);
        let events = self.transformer.transform(frame, mode, &mut self.ids)?;
        let values = events
            .iter()
            .map(event_json)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Json::List(values))
    }
}

fn text_of(message: &RealtimeMessage) -> Result<&str, MuseProtocolError> {
    match message {
        RealtimeMessage::Text(text) => Ok(text.as_str()),
        RealtimeMessage::Binary(bytes) => {
            from_utf8(bytes).map_err(|_error| MuseProtocolError::FrameNotUtf8)
        }
    }
}

#[must_use]
pub fn is_setup_message(message: &Json) -> bool {
    match message {
        Json::Object(fields) => fields.contains_key("authorization"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => false,
    }
}
