use betterlite_openai::openai_base_config::Object;
use serde::{Deserialize, Serialize};
use serde_json::Number;
use serde_json::value::RawValue;
use serde_with::rust::double_option;

use super::error::DeepgramTranscriptionError;

pub const REPLY_MAX_BYTES: usize = 0x0100_0000;

const DEFAULT_LANGUAGE: &str = "en";

// outer None is a missing member and inner None is a null one, so the two stay apart
type Member<T> = Option<Option<T>>;

#[derive(Debug, Deserialize)]
struct Reply {
    results: Results,
    metadata: Metadata,
}

// only the first channel and its first alternative are read, so a malformed later one stays harmless
#[derive(Debug, Deserialize)]
struct Results {
    channels: Vec<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    duration: Number,
}

#[derive(Debug, Deserialize)]
struct Channel {
    #[serde(default)]
    detected_language: Option<String>,
    alternatives: Vec<Box<RawValue>>,
}

#[derive(Debug, Deserialize)]
struct Alternative {
    #[serde(default, with = "double_option")]
    transcript: Member<String>,
    #[serde(default, with = "double_option")]
    words: Member<Vec<Word>>,
    #[serde(default, with = "double_option")]
    paragraphs: Member<Paragraphs>,
}

#[derive(Debug, Deserialize)]
struct Paragraphs {
    #[serde(default, with = "double_option")]
    transcript: Member<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Word {
    pub word: String,
    pub start: Number,
    pub end: Number,
    #[serde(default, with = "double_option")]
    pub speaker: Member<Number>,
    #[serde(default, with = "double_option")]
    pub punctuated_word: Member<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionTask {
    Transcribe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TranscriptionWord {
    pub word: String,
    pub start: Number,
    pub end: Number,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeepgramTranscription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub task: TranscriptionTask,
    pub language: String,
    pub duration: Number,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<TranscriptionWord>>,
    pub hidden_params: Object,
}

pub fn decode_deepgram_transcript(
    body: &[u8],
) -> Result<DeepgramTranscription, DeepgramTranscriptionError> {
    if body.len() > REPLY_MAX_BYTES {
        return Err(DeepgramTranscriptionError::ReplyTooLarge {
            limit: REPLY_MAX_BYTES,
        });
    }
    project(body).map_err(|source| {
        let text = str::from_utf8(body).map_or_else(
            |_| format!("{} bytes that are not UTF-8", body.len()),
            str::to_owned,
        );
        DeepgramTranscriptionError::Transform {
            body: text,
            source: Box::new(source),
        }
    })
}

fn project(body: &[u8]) -> Result<DeepgramTranscription, DeepgramTranscriptionError> {
    let reply: Reply = serde_json::from_slice(body)?;
    let (channel, alternative) = first_alternative(&reply)?;
    let words = match alternative.words {
        None => None,
        Some(None) => return Err(DeepgramTranscriptionError::NullWords),
        Some(Some(words)) => Some(words),
    };
    let text = select_text(
        alternative.transcript,
        alternative.paragraphs,
        words.as_deref().unwrap_or_default(),
    )?;
    Ok(DeepgramTranscription {
        text,
        task: TranscriptionTask::Transcribe,
        language: channel
            .detected_language
            .filter(|language| !language.is_empty())
            .unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned()),
        duration: reply.metadata.duration,
        words: words.map(public_words),
        hidden_params: serde_json::from_slice(body)?,
    })
}

fn first_alternative(reply: &Reply) -> Result<(Channel, Alternative), DeepgramTranscriptionError> {
    let first_channel = reply
        .results
        .channels
        .first()
        .ok_or(DeepgramTranscriptionError::NoChannel)?;
    let channel: Channel = serde_json::from_str(first_channel.get())?;
    let first_alternative = channel
        .alternatives
        .first()
        .ok_or(DeepgramTranscriptionError::NoAlternative)?;
    let alternative: Alternative = serde_json::from_str(first_alternative.get())?;
    Ok((channel, alternative))
}

// the public word entries drop the speaker and everything else the provider adds
fn public_words(words: Vec<Word>) -> Vec<TranscriptionWord> {
    words
        .into_iter()
        .map(|word| TranscriptionWord {
            word: word.word,
            start: word.start,
            end: word.end,
        })
        .collect()
}

// diarization is on only when the first word carries a speaker member, null included
fn select_text(
    transcript: Member<String>,
    paragraphs: Member<Paragraphs>,
    words: &[Word],
) -> Result<Option<String>, DeepgramTranscriptionError> {
    let diarized = words.first().is_some_and(|word| word.speaker.is_some());
    if !diarized {
        return transcript.ok_or(DeepgramTranscriptionError::NoTranscript);
    }
    match paragraphs {
        Some(None) => Err(DeepgramTranscriptionError::NullParagraphs),
        Some(Some(paragraphs)) => paragraphs
            .transcript
            .ok_or(DeepgramTranscriptionError::NoParagraphTranscript),
        None => reconstruct_diarized_text(words).map(Some),
    }
}

// the final run gets a leading and trailing newline, so the last two runs sit a blank line apart
pub fn reconstruct_diarized_text(words: &[Word]) -> Result<String, DeepgramTranscriptionError> {
    // a word without a speaker prints as None, the way LiteLLM's f-string does
    let label =
        |speaker: Option<&Number>| speaker.map_or_else(|| "None".to_owned(), Number::to_string);
    let mut segments: Vec<String> = Vec::new();
    let mut current_speaker: Option<&Number> = None;
    let mut current_words: Vec<&str> = Vec::new();
    for word in words {
        let speaker = word.speaker.as_ref().and_then(Option::as_ref);
        let text = match &word.punctuated_word {
            None => word.word.as_str(),
            Some(None) => return Err(DeepgramTranscriptionError::NullPunctuatedWord),
            Some(Some(punctuated)) => punctuated.as_str(),
        };
        if speaker == current_speaker {
            current_words.push(text);
            continue;
        }
        if !current_words.is_empty() {
            segments.push(format!(
                "Speaker {}: {}",
                label(current_speaker),
                current_words.join(" ")
            ));
        }
        current_speaker = speaker;
        current_words = vec![text];
    }
    if !current_words.is_empty() {
        segments.push(format!(
            "\nSpeaker {}: {}\n",
            label(current_speaker),
            current_words.join(" ")
        ));
    }
    Ok(segments.join("\n"))
}
