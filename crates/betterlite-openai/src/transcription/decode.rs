use std::str::from_utf8;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::error::TranscriptionError;
use crate::api::transcription_contract::TranscriptionResponse;
use crate::openai_base_config::Object;

// The OpenAI SDK model the reply is parsed into, picked by the request's response_format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyFormat {
    Transcription,
    Verbose,
    Diarized,
    Text,
}

// How the SDK builds a declared field's value; anything not of that shape is kept as sent.
#[derive(Debug, Clone, Copy)]
enum Shape {
    Plain,
    Item(Model),
    Items(Model),
    // the tokens or duration usage union; any type other than duration builds the tokens variant
    Usage,
}

type Model = &'static [(&'static str, Shape)];

const LANGUAGE: Model = &[("code", Shape::Plain)];
const LOGPROB: Model = &[
    ("token", Shape::Plain),
    ("bytes", Shape::Plain),
    ("logprob", Shape::Plain),
];
const TOKEN_DETAILS: Model = &[
    ("audio_tokens", Shape::Plain),
    ("text_tokens", Shape::Plain),
];
const TOKENS: Model = &[
    ("input_tokens", Shape::Plain),
    ("output_tokens", Shape::Plain),
    ("total_tokens", Shape::Plain),
    ("type", Shape::Plain),
    ("input_token_details", Shape::Item(TOKEN_DETAILS)),
];
const DURATION: Model = &[("seconds", Shape::Plain), ("type", Shape::Plain)];
const TRANSCRIPTION: Model = &[
    ("text", Shape::Plain),
    ("languages", Shape::Items(LANGUAGE)),
    ("logprobs", Shape::Items(LOGPROB)),
    ("usage", Shape::Usage),
];
const SEGMENT: Model = &[
    ("id", Shape::Plain),
    ("avg_logprob", Shape::Plain),
    ("compression_ratio", Shape::Plain),
    ("end", Shape::Plain),
    ("no_speech_prob", Shape::Plain),
    ("seek", Shape::Plain),
    ("start", Shape::Plain),
    ("temperature", Shape::Plain),
    ("text", Shape::Plain),
    ("tokens", Shape::Plain),
];
const WORD: Model = &[
    ("end", Shape::Plain),
    ("start", Shape::Plain),
    ("word", Shape::Plain),
];
const VERBOSE: Model = &[
    ("duration", Shape::Plain),
    ("language", Shape::Plain),
    ("text", Shape::Plain),
    ("segments", Shape::Items(SEGMENT)),
    ("usage", Shape::Item(DURATION)),
    ("words", Shape::Items(WORD)),
];
const DIARIZED_SEGMENT: Model = &[
    ("id", Shape::Plain),
    ("end", Shape::Plain),
    ("speaker", Shape::Plain),
    ("start", Shape::Plain),
    ("text", Shape::Plain),
    ("type", Shape::Plain),
];
const DIARIZED: Model = &[
    ("duration", Shape::Plain),
    ("segments", Shape::Items(DIARIZED_SEGMENT)),
    ("task", Shape::Plain),
    ("text", Shape::Plain),
    ("usage", Shape::Usage),
];

// The SDK's model_dump: every declared field in order, null when the body has none, then the extra keys.
fn sdk_dump(model: Model, fields: &IndexMap<String, Json>) -> IndexMap<String, Json> {
    let mut dumped = IndexMap::new();
    for (name, shape) in model {
        let value = fields
            .get(*name)
            .map_or(Json::Null, |value| shaped(*shape, value));
        dumped.insert((*name).to_owned(), value);
    }
    for (name, value) in fields {
        dumped.entry(name.clone()).or_insert_with(|| value.clone());
    }
    dumped
}

fn shaped(shape: Shape, value: &Json) -> Json {
    match (shape, value) {
        (Shape::Item(model), Json::Object(fields)) => Json::Object(sdk_dump(model, fields)),
        (Shape::Items(model), Json::List(items)) => Json::List(
            items
                .iter()
                .map(|item| shaped(Shape::Item(model), item))
                .collect(),
        ),
        (Shape::Usage, Json::Object(fields)) => {
            let duration =
                matches!(fields.get("type"), Some(Json::Text(kind)) if kind == "duration");
            Json::Object(sdk_dump(if duration { DURATION } else { TOKENS }, fields))
        }
        _ => value.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedReply {
    pub response: TranscriptionResponse,
    pub original_response: Json,
    // the SDK parsed the body as plain text, not as a JSON object
    pub textual: bool,
}

pub fn reply_format(options: &IndexMap<String, Json>) -> Result<ReplyFormat, TranscriptionError> {
    Ok(match string_field(options, "response_format")? {
        Some("text" | "srt" | "vtt") => ReplyFormat::Text,
        Some("verbose_json") => ReplyFormat::Verbose,
        Some("diarized_json") => ReplyFormat::Diarized,
        Some(_) | None => ReplyFormat::Transcription,
    })
}

// decode_transcription_reply is the one place provider reply bytes turn into a TranscriptionResponse.
pub fn decode_transcription_reply(
    body: &[u8],
    content_type: Option<&str>,
    format: ReplyFormat,
) -> Result<DecodedReply, TranscriptionError> {
    let sdk_model = match format {
        ReplyFormat::Transcription => Some(TRANSCRIPTION),
        ReplyFormat::Verbose => Some(VERBOSE),
        ReplyFormat::Diarized => Some(DIARIZED),
        ReplyFormat::Text => None,
    };
    if let Some(model) = sdk_model {
        match serde_json::from_slice::<Json>(body) {
            Ok(Json::Object(fields)) => {
                let dumped = sdk_dump(model, &fields);
                return Ok(DecodedReply {
                    response: response_from_object(&dumped)?,
                    original_response: Json::Object(dumped),
                    textual: false,
                });
            }
            Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
                return Err(TranscriptionError::NotObject);
            }
            // the SDK's own branch: the part before any parameter must end in "json"
            Err(source) => {
                let base = content_type.and_then(|kind| kind.split(';').next());
                if base.is_some_and(|kind| kind.trim().ends_with("json")) {
                    return Err(TranscriptionError::Json { source });
                }
            }
        }
    }
    let text = from_utf8(body).map_err(|source| TranscriptionError::Utf8 { source })?;
    Ok(DecodedReply {
        response: TranscriptionResponse {
            text: Some(text.to_owned()),
            extra: Object::new(),
        },
        original_response: Json::Object(IndexMap::from([
            (String::from("text"), Json::Text(text.to_owned())),
            (String::from("usage"), Json::Null),
        ])),
        textual: true,
    })
}

pub fn string_field<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &str,
) -> Result<Option<&'fields str>, TranscriptionError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(TranscriptionError::Field {
                field: name.to_owned(),
                problem: "is not a string",
            })
        }
    }
}

// convert_to_model_response_object keeps these response fields and a usage whose type is duration or tokens.
fn response_from_object(
    fields: &IndexMap<String, Json>,
) -> Result<TranscriptionResponse, TranscriptionError> {
    let mut extra = Object::new();
    for name in ["language", "task", "duration", "words", "segments"] {
        // a field the dump filled with null is None in LiteLLM's response, and absent here
        if let Some(value) = fields.get(name).filter(|value| **value != Json::Null) {
            let kept = serde_json::to_value(value)
                .map_err(|source| TranscriptionError::Json { source })?;
            extra.insert(String::from(name), kept);
        }
    }
    match fields.get("usage") {
        None | Some(Json::Null) => {}
        Some(usage @ Json::Object(parts)) => {
            if matches!(parts.get("type"), Some(Json::Text(kind)) if kind == "duration" || kind == "tokens")
            {
                let kept = serde_json::to_value(usage)
                    .map_err(|source| TranscriptionError::Json { source })?;
                extra.insert(String::from("usage"), kept);
            }
        }
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(TranscriptionError::Field {
                field: String::from("usage"),
                problem: "is not an object",
            });
        }
    }
    Ok(TranscriptionResponse {
        text: string_field(fields, "text")?.map(str::to_owned),
        extra,
    })
}
