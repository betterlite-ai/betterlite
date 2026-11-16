use betterlite_prompts::arguments::json::{Json, Object};
use serde::Deserialize;

use super::decode::{Audio, ClientEvent, ItemCreate, OptionalOutput, ToolResult};
use super::error::LiveClientError;
use super::records::{
    ClientContent, ClientContentMessage, FunctionResponse, RealtimeInputMessage, TextPart,
    ToolResponse, ToolResponseMessage, Turn,
};
use super::state::LiveSessionState;
use super::turn::manual_turn_detection;
use crate::gemini_live_setup::settings::LiveSetupSettings;
use crate::gemini_live_setup::setup::transform_session_update;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FunctionResponseId {
    #[default]
    Include,
    Omit,
}

#[derive(Debug)]
pub struct LiveClient {
    pub state: LiveSessionState,
    pub function_response_id: FunctionResponseId,
}

impl LiveClient {
    pub fn transform_realtime_request(
        &mut self,
        event: &ClientEvent,
        model: &str,
        session_configuration_request: Option<&str>,
        setup: &LiveSetupSettings<'_>,
    ) -> Result<Vec<String>, LiveClientError> {
        match event {
            ClientEvent::SessionUpdate(session) => Ok(transform_session_update(
                session,
                model,
                session_configuration_request,
                setup,
            )?),
            ClientEvent::ResponseCreate | ClientEvent::Unknown | ClientEvent::AudioClear => {
                Ok(Vec::new())
            }
            ClientEvent::AudioAppend(audio) => self.audio_append(audio),
            ClientEvent::AudioCommit | ClientEvent::AudioEnd => {
                let key = if manual_turn_detection(session_configuration_request) {
                    "activityEnd"
                } else {
                    "audioStreamEnd"
                };
                let mut realtime_input = Object::new();
                realtime_input.insert(key.to_owned(), Json::Bool(true));
                let message = RealtimeInputMessage { realtime_input };
                let encoded = serde_json::to_string(&message).map_err(LiveClientError::Encode)?;
                Ok(vec![encoded])
            }
            ClientEvent::ItemCreate(item) => self.conversation_item(item),
        }
    }

    fn audio_append(&mut self, audio: &Audio) -> Result<Vec<String>, LiveClientError> {
        let data = match audio {
            Audio::Missing => return Err(LiveClientError::MissingAudio),
            Audio::Null => None,
            Audio::Data(text) => {
                self.state.unbilled_input_audio_bytes = self
                    .state
                    .unbilled_input_audio_bytes
                    .saturating_add(base64_byte_estimate(text));
                Some(text.as_str())
            }
        };
        let mut blob = Object::new();
        blob.insert(
            "mimeType".to_owned(),
            Json::Text(audio_mime_type("pcm16").to_owned()),
        );
        blob.insert(
            "data".to_owned(),
            data.map_or(Json::Null, |text| Json::Text(text.to_owned())),
        );
        let mut realtime_input = Object::new();
        realtime_input.insert("audio".to_owned(), Json::Object(blob));
        let message = RealtimeInputMessage { realtime_input };
        let encoded = serde_json::to_string(&message).map_err(LiveClientError::Encode)?;
        Ok(vec![encoded])
    }

    fn conversation_item(&mut self, item: &ItemCreate) -> Result<Vec<String>, LiveClientError> {
        match item {
            ItemCreate::ToolResult(result) => self.tool_response(result),
            ItemCreate::UserText { text } => {
                if text.is_empty() {
                    return Ok(Vec::new());
                }
                let message = ClientContentMessage {
                    client_content: ClientContent {
                        turns: vec![Turn {
                            role: "user".to_owned(),
                            parts: vec![TextPart { text: text.clone() }],
                        }],
                        turn_complete: true,
                    },
                };
                let encoded = serde_json::to_string(&message).map_err(LiveClientError::Encode)?;
                Ok(vec![encoded])
            }
        }
    }

    fn tool_response(&mut self, result: &ToolResult) -> Result<Vec<String>, LiveClientError> {
        let call_id = result.call_id.as_deref();
        let function_name = call_id
            .and_then(|id| self.state.tool_call_names.entries.get(id))
            .filter(|name| !name.is_empty())
            .cloned();
        if function_name.is_some()
            && let Some(id) = call_id
        {
            self.state.tool_call_names.refresh(id);
        } else {
            tracing::warn!("function name not found for call_id; Gemini may reject the tool reply");
        }
        let response = FunctionResponse {
            response: output_object(result.output.clone()),
            id: match self.function_response_id {
                FunctionResponseId::Include => {
                    call_id.filter(|id| !id.is_empty()).map(str::to_owned)
                }
                FunctionResponseId::Omit => None,
            },
            name: function_name,
        };
        let message = ToolResponseMessage {
            tool_response: ToolResponse {
                function_responses: vec![response],
            },
        };
        let encoded = serde_json::to_string(&message).map_err(LiveClientError::Encode)?;
        Ok(vec![encoded])
    }
}

fn output_object(output: OptionalOutput) -> Object {
    let value = match output {
        None => return Object::new(),
        Some(None) => Json::Null,
        // LiteLLM parses a string output as JSON and keeps the text when it is not JSON
        Some(Some(Json::Text(text))) => {
            serde_json::from_str::<Json>(&text).unwrap_or_else(|_| Json::Text(text))
        }
        Some(Some(value)) => value,
    };
    if let Json::Object(object) = value {
        return object;
    }
    let mut object = Object::new();
    object.insert("result".to_owned(), value);
    object
}

fn base64_byte_estimate(data: &str) -> u64 {
    let length = u64::try_from(data.chars().count()).unwrap_or(u64::MAX);
    let padding = if data.ends_with("==") {
        2
    } else {
        u64::from(data.ends_with('='))
    };
    length
        .saturating_mul(3)
        .saturating_div(4)
        .saturating_sub(padding)
}

#[must_use]
pub fn audio_mime_type(input_audio_format: &str) -> &'static str {
    match input_audio_format {
        "pcm16" => "audio/pcm;rate=24000",
        "g711_ulaw" => "audio/pcmu",
        "g711_alaw" => "audio/pcma",
        _ => "application/octet-stream",
    }
}
