use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::Number;

use super::RealtimeSession;
use super::records::{
    ContentEnd, ContentEndEvent, ContentEndFrame, ContentRole, ContentStart, ContentStartEvent,
    ContentStartFrame, ContentType, ExactDecimal, MaxResponseTokens, Setting, TextInput,
    TextInputConfiguration, TextInputEvent, TextInputFrame, UpdateFrame, nullable_setting,
};
use super::tools::ToolDefinition;
use crate::agentcore_stream::IdSource;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionUpdate {
    #[serde(default)]
    pub session: SessionConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionConfig {
    #[serde(default, deserialize_with = "nullable_setting")]
    pub max_response_output_tokens: Option<Setting<MaxResponseTokens>>,
    #[serde(default, deserialize_with = "temperature_setting")]
    pub temperature: Option<Setting<ExactDecimal>>,
    #[serde(default, deserialize_with = "nullable_setting")]
    pub voice: Option<Setting<String>>,
    #[serde(default, deserialize_with = "nullable_setting")]
    pub output_audio_format: Option<Setting<String>>,
    #[serde(default, deserialize_with = "nullable_setting")]
    pub input_audio_format: Option<Setting<String>>,
    #[serde(default, deserialize_with = "nullable_setting")]
    pub output_sample_rate_hertz: Option<Setting<i64>>,
    #[serde(default, deserialize_with = "nullable_setting")]
    pub input_sample_rate_hertz: Option<Setting<i64>>,
    #[serde(default)]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(default)]
    pub instructions: Option<Setting<String>>,
}

pub fn temperature_setting<'de, D>(
    deserializer: D,
) -> Result<Option<Setting<ExactDecimal>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<Number>::deserialize(deserializer)?.map_or_else(
        || Ok(Some(Setting(None))),
        |number| {
            parse_decimal_lexeme(&number.to_string())
                .map(|value| Some(Setting(Some(ExactDecimal(value)))))
                .map_err(D::Error::custom)
        },
    )
}

pub(super) fn instruction_frames(
    session: &RealtimeSession,
    instructions: Option<&Setting<String>>,
    ids: &dyn IdSource,
) -> Vec<UpdateFrame> {
    let Some(text) = instruction_text(instructions) else {
        return Vec::new();
    };
    let content_name = ids.next_id().to_string();
    vec![
        UpdateFrame::ContentStart(ContentStartFrame {
            event: ContentStartEvent {
                content_start: ContentStart {
                    prompt_name: session.names.prompt.clone(),
                    content_name: content_name.clone(),
                    content_type: ContentType::Text,
                    interactive: false,
                    role: ContentRole::System,
                    text_input_configuration: TextInputConfiguration {
                        media_type: session.text_media_type.clone(),
                    },
                },
            },
        }),
        UpdateFrame::TextInput(TextInputFrame {
            event: TextInputEvent {
                text_input: TextInput {
                    prompt_name: session.names.prompt.clone(),
                    content_name: content_name.clone(),
                    content: text.to_owned(),
                },
            },
        }),
        UpdateFrame::ContentEnd(ContentEndFrame {
            event: ContentEndEvent {
                content_end: ContentEnd {
                    prompt_name: session.names.prompt.clone(),
                    content_name,
                },
            },
        }),
    ]
}

const fn instruction_text(instructions: Option<&Setting<String>>) -> Option<&str> {
    match instructions {
        Some(Setting(Some(text))) if !text.is_empty() => Some(text.as_str()),
        _ => None,
    }
}
