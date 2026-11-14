pub mod records;
pub mod tools;
pub mod update;

use rust_decimal::Decimal;

use crate::agentcore_stream::IdSource;
use records::{
    AUDIO_MEDIA_TYPE, AudioOutputConfiguration, AudioSettings, ContentEnd, ContentEndEvent,
    ContentEndFrame, ContentRole, ContentStart, ContentStartEvent, ContentStartFrame, ContentType,
    ExactDecimal, InferenceConfiguration, MaxResponseTokens, MediaType, PromptStart,
    PromptStartEvent, PromptStartFrame, SessionConfiguration, SessionEvent, SessionEventType,
    SessionObject, SessionStart, SessionStartEvent, SessionStartFrame, Setting, TEXT_MEDIA_TYPE,
    TOOL_MEDIA_TYPE, TextInput, TextInputConfiguration, TextInputEvent, TextInputFrame,
    ToolConfiguration, UpdateFrame, UsageEvent,
};
use update::{SessionConfig, SessionUpdate};

mod tests;

pub const REQUIRES_SESSION_CONFIGURATION: bool = true;

pub struct SessionNames {
    pub prompt: String,
    pub content: String,
    pub audio_content: String,
}

pub struct TranscriptState {
    pub active: bool,
    pub generation_stage: Option<String>,
    pub item_id: Option<String>,
    pub buffer: String,
}

pub struct RealtimeSession {
    pub names: SessionNames,
    pub prompt_started: bool,
    pub client_audio_streamed: bool,
    pub max_tokens: Setting<MaxResponseTokens>,
    pub top_p: ExactDecimal,
    pub temperature: Setting<ExactDecimal>,
    pub voice_id: Setting<String>,
    pub output_audio: AudioSettings,
    pub input_audio: AudioSettings,
    pub text_media_type: String,
    pub transcript: TranscriptState,
    pub cumulative_usage: UsageEvent,
    pub reported_usage: UsageEvent,
}

impl RealtimeSession {
    #[must_use]
    pub fn new(ids: &dyn IdSource) -> Self {
        Self {
            names: SessionNames {
                prompt: ids.next_id().to_string(),
                content: ids.next_id().to_string(),
                audio_content: ids.next_id().to_string(),
            },
            prompt_started: false,
            client_audio_streamed: false,
            max_tokens: Setting(Some(MaxResponseTokens::Count(1024))),
            top_p: ExactDecimal(Decimal::new(9, 1)),
            temperature: Setting(Some(ExactDecimal(Decimal::new(7, 1)))),
            voice_id: Setting(Some("matthew".to_owned())),
            output_audio: AudioSettings {
                sample_rate_hertz: Setting(Some(24000)),
                sample_size_bits: 16,
                channel_count: 1,
                encoding: "base64".to_owned(),
                audio_type: "SPEECH".to_owned(),
                media_type: AUDIO_MEDIA_TYPE.to_owned(),
            },
            input_audio: AudioSettings {
                sample_rate_hertz: Setting(Some(16000)),
                sample_size_bits: 16,
                channel_count: 1,
                encoding: "base64".to_owned(),
                audio_type: "SPEECH".to_owned(),
                media_type: AUDIO_MEDIA_TYPE.to_owned(),
            },
            text_media_type: TEXT_MEDIA_TYPE.to_owned(),
            transcript: TranscriptState {
                active: false,
                generation_stage: None,
                item_id: None,
                buffer: String::new(),
            },
            cumulative_usage: UsageEvent::default(),
            reported_usage: UsageEvent::default(),
        }
    }

    #[must_use]
    pub fn configuration_request(
        &self,
        tools: Option<&[tools::ToolDefinition]>,
    ) -> SessionConfiguration {
        SessionConfiguration {
            session_start: SessionStartEvent {
                session_start: self.session_start(),
            },
            prompt_start: PromptStartEvent {
                prompt_start: self.prompt_start(tools),
            },
        }
    }

    #[must_use]
    pub fn apply_update(&mut self, update: &SessionUpdate, ids: &dyn IdSource) -> Vec<UpdateFrame> {
        self.apply_settings(&update.session);
        let mut frames = vec![
            UpdateFrame::SessionStart(SessionStartFrame {
                event: SessionStartEvent {
                    session_start: self.session_start(),
                },
            }),
            UpdateFrame::PromptStart(PromptStartFrame {
                event: PromptStartEvent {
                    prompt_start: self.prompt_start(update.session.tools.as_deref()),
                },
            }),
        ];
        self.prompt_started = true;
        let Some(text) = instruction_text(update.session.instructions.as_ref()) else {
            return frames;
        };
        let content_name = ids.next_id().to_string();
        frames.push(UpdateFrame::ContentStart(ContentStartFrame {
            event: ContentStartEvent {
                content_start: ContentStart {
                    prompt_name: self.names.prompt.clone(),
                    content_name: content_name.clone(),
                    content_type: ContentType::Text,
                    interactive: false,
                    role: ContentRole::System,
                    text_input_configuration: TextInputConfiguration {
                        media_type: self.text_media_type.clone(),
                    },
                },
            },
        }));
        frames.push(UpdateFrame::TextInput(TextInputFrame {
            event: TextInputEvent {
                text_input: TextInput {
                    prompt_name: self.names.prompt.clone(),
                    content_name: content_name.clone(),
                    content: text.to_owned(),
                },
            },
        }));
        frames.push(UpdateFrame::ContentEnd(ContentEndFrame {
            event: ContentEndEvent {
                content_end: ContentEnd {
                    prompt_name: self.names.prompt.clone(),
                    content_name,
                },
            },
        }));
        frames
    }

    #[must_use]
    pub fn session_created_event(
        model: Option<&str>,
        trace_id: &str,
        ids: &dyn IdSource,
    ) -> SessionEvent {
        let modalities = vec!["text".to_owned(), "audio".to_owned()];
        SessionEvent {
            event_id: ids.next_id().to_string(),
            session: SessionObject {
                id: trace_id.to_owned(),
                modalities,
                model: model.map(str::to_owned),
            },
            event_type: SessionEventType::Created,
        }
    }

    #[must_use]
    pub fn session_updated_event(
        model: Option<&str>,
        trace_id: &str,
        modalities: Option<&[String]>,
        ids: &dyn IdSource,
    ) -> SessionEvent {
        let modalities = match modalities {
            Some(values) => values.to_vec(),
            None => vec!["text".to_owned(), "audio".to_owned()],
        };
        SessionEvent {
            event_id: ids.next_id().to_string(),
            session: SessionObject {
                id: trace_id.to_owned(),
                modalities,
                model: model.map(str::to_owned),
            },
            event_type: SessionEventType::Updated,
        }
    }

    fn apply_settings(&mut self, session: &SessionConfig) {
        if let Some(tokens) = &session.max_response_output_tokens {
            self.max_tokens = tokens.clone();
        }
        if let Some(temperature) = &session.temperature {
            self.temperature = temperature.clone();
        }
        if let Some(voice) = &session.voice {
            self.voice_id = voice.clone();
        }
        if let Some(format) = &session.output_audio_format {
            self.output_audio.sample_rate_hertz = Setting(Some(audio_rate(format, true)));
        }
        if let Some(format) = &session.input_audio_format {
            self.input_audio.sample_rate_hertz = Setting(Some(audio_rate(format, false)));
        }
        if let Some(rate) = &session.output_sample_rate_hertz {
            self.output_audio.sample_rate_hertz = rate.clone();
        }
        if let Some(rate) = &session.input_sample_rate_hertz {
            self.input_audio.sample_rate_hertz = rate.clone();
        }
    }

    fn session_start(&self) -> SessionStart {
        SessionStart {
            inference_configuration: InferenceConfiguration {
                max_tokens: self.max_tokens.clone(),
                top_p: self.top_p.clone(),
                temperature: self.temperature.clone(),
            },
        }
    }

    fn prompt_start(&self, tools: Option<&[tools::ToolDefinition]>) -> PromptStart {
        let (tool_use_output_configuration, tool_configuration) = match tools {
            Some(tools) if !tools.is_empty() => (
                Some(MediaType {
                    media_type: TOOL_MEDIA_TYPE.to_owned(),
                }),
                Some(ToolConfiguration {
                    tools: tools::transform(tools),
                }),
            ),
            Some(_) | None => (None, None),
        };
        PromptStart {
            prompt_name: self.names.prompt.clone(),
            text_output_configuration: MediaType {
                media_type: self.text_media_type.clone(),
            },
            audio_output_configuration: AudioOutputConfiguration {
                media_type: self.output_audio.media_type.clone(),
                sample_rate_hertz: self.output_audio.sample_rate_hertz.clone(),
                sample_size_bits: self.output_audio.sample_size_bits,
                channel_count: self.output_audio.channel_count,
                voice_id: self.voice_id.clone(),
                encoding: self.output_audio.encoding.clone(),
                audio_type: self.output_audio.audio_type.clone(),
            },
            tool_use_output_configuration,
            tool_configuration,
        }
    }
}

fn instruction_text(instructions: Option<&Setting<String>>) -> Option<&str> {
    match instructions {
        Some(Setting(Some(text))) if !text.is_empty() => Some(text),
        Some(Setting(Some(_) | None)) | None => None,
    }
}

fn audio_rate(format: &Setting<String>, is_output: bool) -> i64 {
    match format {
        Setting(Some(name)) if name == "g711_ulaw" || name == "g711_alaw" => 8000,
        Setting(Some(_) | None) => default_rate(is_output),
    }
}

const fn default_rate(is_output: bool) -> i64 {
    if is_output { 24000 } else { 16000 }
}

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    match api_base {
        Some(base) => base.to_owned(),
        None => String::new(),
    }
}
