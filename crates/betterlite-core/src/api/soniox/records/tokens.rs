use betterlite_openai::api::audio_subtitles::{Speaker, SubtitleToken};
use serde::Deserialize;

pub const TRANSLATION_STATUS: &str = "translation";

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum Milliseconds {
    Flag(bool),
    Number(i64),
}

impl Milliseconds {
    #[must_use]
    pub const fn value(self) -> i64 {
        match self {
            Self::Flag(flag) => i64::from(flag),
            Self::Number(number) => number,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum SpeakerValue {
    Flag(bool),
    Number(i64),
    Text(String),
}

impl SpeakerValue {
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Flag(flag) => {
                if *flag {
                    "True".to_owned()
                } else {
                    "False".to_owned()
                }
            }
            Self::Number(number) => number.to_string(),
            Self::Text(text) => text.clone(),
        }
    }

    #[must_use]
    pub fn subtitle(&self) -> Speaker {
        match self {
            Self::Flag(flag) => Speaker::Number(i64::from(*flag)),
            Self::Number(number) => Speaker::Number(*number),
            Self::Text(text) => Speaker::Text(text.clone()),
        }
    }
}

impl PartialEq for SpeakerValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Flag(left), Self::Flag(right)) => left == right,
            (Self::Flag(flag), Self::Number(number)) | (Self::Number(number), Self::Flag(flag)) => {
                i64::from(*flag) == *number
            }
            (Self::Number(left), Self::Number(right)) => left == right,
            (Self::Text(left), Self::Text(right)) => left == right,
            (Self::Flag(_) | Self::Number(_), Self::Text(_))
            | (Self::Text(_), Self::Flag(_) | Self::Number(_)) => false,
        }
    }
}

impl Eq for SpeakerValue {}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SonioxToken {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub start_ms: Option<Milliseconds>,
    #[serde(default)]
    pub end_ms: Option<Milliseconds>,
    #[serde(default)]
    pub speaker: Option<SpeakerValue>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub translation_status: Option<String>,
}

impl SonioxToken {
    #[must_use]
    pub fn is_translation(&self) -> bool {
        self.translation_status.as_deref() == Some(TRANSLATION_STATUS)
    }
}

#[must_use]
pub fn render_soniox_tokens(tokens: &[SonioxToken]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut current_speaker: Option<&SpeakerValue> = None;
    let mut current_language: Option<&str> = None;
    for token in tokens {
        let mut text = token.text.clone().unwrap_or_default();
        let speaker = token
            .speaker
            .as_ref()
            .filter(|speaker| current_speaker != Some(*speaker));
        if let Some(speaker) = speaker {
            if current_speaker.is_some() {
                parts.push(String::from("\n\n"));
            }
            current_speaker = Some(speaker);
            current_language = None;
            parts.push(format!("Speaker {}:", speaker.label()));
        }
        let language = token
            .language
            .as_deref()
            .filter(|language| current_language != Some(*language));
        if let Some(language) = language {
            current_language = Some(language);
            let prefix = if token.is_translation() {
                "[Translation] "
            } else {
                ""
            };
            parts.push(format!("\n{prefix}[{language}] "));
            text = text.trim_start_matches(is_python_whitespace).to_owned();
        }
        parts.push(text);
    }
    parts.concat()
}

fn is_python_whitespace(character: char) -> bool {
    if character.is_whitespace() {
        return true;
    }
    matches!(character, '\u{1c}'..='\u{1f}')
}

pub(super) fn subtitle_tokens(tokens: &[SonioxToken]) -> Vec<SubtitleToken> {
    let mut kept: Vec<SubtitleToken> = Vec::new();
    for token in tokens {
        if token.is_translation() {
            continue;
        }
        kept.push(SubtitleToken {
            text: token.text.clone().unwrap_or_default(),
            start_ms: token.start_ms.map(Milliseconds::value),
            end_ms: token.end_ms.map(Milliseconds::value),
            speaker: token.speaker.as_ref().map(SpeakerValue::subtitle),
        });
    }
    kept
}
