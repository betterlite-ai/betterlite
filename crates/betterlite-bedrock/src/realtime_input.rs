pub mod conversation;
pub mod decode;
pub mod error;
pub mod frames;
pub mod trigger;

use std::collections::BTreeMap;

use crate::agentcore_stream::IdSource;
use crate::realtime_session::RealtimeSession;
use crate::realtime_session::records::{ContentEnd, Setting};
use decode::{ClientEvent, decode_realtime_client_input};
use error::InputError;
use frames::{
    AudioInput, AudioInputConfiguration, ContentRole, ContentStart, ContentType, Event,
    InputConfiguration, InputFrame, PromptEnd,
};
use trigger::{TRIGGER_SAMPLE_RATE_HERTZ, trigger_audio_frames};

mod tests;

pub struct RealtimeInput {
    pub session: RealtimeSession,
    // The sample rate of the audio content that is open on the Bedrock stream, absent when none is
    pub open_audio: Option<Setting<i64>>,
}

impl RealtimeInput {
    pub fn transform_realtime_request(
        &mut self,
        message: &[u8],
        ids: &dyn IdSource,
    ) -> Result<Vec<InputFrame>, InputError> {
        let event = match decode_realtime_client_input(message) {
            Ok(event) => event,
            Err(InputError::InvalidJson { .. }) => {
                // The raw text stays out of the log, since it can hold client audio or text
                tracing::warn!("Invalid JSON message");
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        Ok(match event {
            ClientEvent::SessionUpdate(update) => self
                .session
                .apply_update(&update, ids)
                .into_iter()
                .map(InputFrame::Session)
                .collect(),
            ClientEvent::AudioAppend(audio) => self.audio_append(audio, ids),
            ClientEvent::AudioCommit => {
                tracing::debug!("Handling input_audio_buffer.commit");
                self.end_audio().into_iter().collect()
            }
            ClientEvent::ItemCreate(item) => conversation::item_frames(&self.session, item, ids),
            ClientEvent::ResponseCreate => self.response_create(),
            ClientEvent::ResponseCancel => {
                tracing::debug!("Handling response.cancel");
                Vec::new()
            }
            ClientEvent::Unknown(message_type) => {
                tracing::warn!("Unknown message type: {message_type}");
                Vec::new()
            }
        })
    }

    pub fn session_close_messages(&mut self) -> Vec<InputFrame> {
        if !self.session.prompt_started {
            return Vec::new();
        }
        let mut frames: Vec<InputFrame> = self.end_audio().into_iter().collect();
        frames.push(InputFrame::from(Event::PromptEnd(PromptEnd {
            prompt_name: self.session.names.prompt.clone(),
        })));
        frames.push(InputFrame::from(Event::SessionEnd(BTreeMap::new())));
        self.session.prompt_started = false;
        frames
    }

    fn audio_append(&mut self, audio: Setting<String>, ids: &dyn IdSource) -> Vec<InputFrame> {
        tracing::debug!("Handling input_audio_buffer.append");
        self.session.client_audio_streamed = true;
        let rate = self.session.input_audio.sample_rate_hertz.clone();
        let mut frames = Vec::new();
        if self.open_audio.as_ref().is_some_and(|open| *open != rate) {
            frames.extend(self.end_audio());
            self.session.names.audio_content = ids.next_id().to_string();
        }
        if self.open_audio.is_none() {
            frames.push(self.start_audio(rate));
        }
        frames.push(InputFrame::from(Event::AudioInput(AudioInput {
            prompt_name: self.session.names.prompt.clone(),
            content_name: self.session.names.audio_content.clone(),
            content: audio,
        })));
        frames
    }

    fn response_create(&mut self) -> Vec<InputFrame> {
        tracing::debug!("Handling response.create");
        if !self.session.prompt_started || self.session.client_audio_streamed {
            return Vec::new();
        }
        let mut frames = Vec::new();
        if self.open_audio.is_none() {
            frames.push(self.start_audio(Setting(Some(TRIGGER_SAMPLE_RATE_HERTZ))));
        }
        frames.extend(trigger_audio_frames(
            &self.session.names.prompt,
            &self.session.names.audio_content,
        ));
        frames
    }

    fn start_audio(&mut self, rate: Setting<i64>) -> InputFrame {
        let audio = &self.session.input_audio;
        let start = ContentStart {
            prompt_name: self.session.names.prompt.clone(),
            content_name: self.session.names.audio_content.clone(),
            content_type: ContentType::Audio,
            interactive: true,
            role: ContentRole::User,
            configuration: InputConfiguration::AudioInputConfiguration(AudioInputConfiguration {
                media_type: audio.media_type.clone(),
                sample_rate_hertz: rate.clone(),
                sample_size_bits: audio.sample_size_bits,
                channel_count: audio.channel_count,
                audio_type: audio.audio_type.clone(),
                encoding: audio.encoding.clone(),
            }),
        };
        self.open_audio = Some(rate);
        InputFrame::from(Event::ContentStart(start))
    }

    fn end_audio(&mut self) -> Option<InputFrame> {
        self.open_audio.take()?;
        Some(InputFrame::from(Event::ContentEnd(ContentEnd {
            prompt_name: self.session.names.prompt.clone(),
            content_name: self.session.names.audio_content.clone(),
        })))
    }
}
