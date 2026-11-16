use std::borrow::Cow;
use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::error::Category;
use serde_json::value::RawValue;

use super::error::LiveClientError;
use crate::gemini_live_setup::session::SessionUpdate;

#[derive(Debug)]
pub enum ClientEvent {
    SessionUpdate(Box<SessionUpdate>),
    ResponseCreate,
    ItemCreate(ItemCreate),
    AudioAppend(Audio),
    AudioCommit,
    AudioEnd,
    AudioClear,
    Unknown,
}

// a null audio value is forwarded as JSON null, while a missing one is an error
#[derive(Debug, PartialEq, Eq)]
pub enum Audio {
    Missing,
    Null,
    Data(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ItemCreate {
    ToolResult(ToolResult),
    UserText { text: String },
}

#[derive(Debug, PartialEq, Eq)]
pub struct ToolResult {
    pub call_id: Option<String>,
    // absent wraps into an empty response, null into a null result, and a string is parsed as JSON
    pub output: Option<Option<String>>,
}

#[derive(Deserialize)]
enum KnownType {
    #[serde(rename = "session.update")]
    SessionUpdate,
    #[serde(rename = "response.create")]
    ResponseCreate,
    #[serde(rename = "conversation.item.create")]
    ItemCreate,
    #[serde(rename = "input_audio_buffer.append")]
    AudioAppend,
    #[serde(rename = "input_audio_buffer.commit")]
    AudioCommit,
    #[serde(rename = "input_audio_buffer.end")]
    AudioEnd,
    #[serde(rename = "input_audio_buffer.clear")]
    AudioClear,
}

#[derive(Deserialize)]
enum ItemType {
    #[serde(rename = "function_call_output")]
    FunctionCallOutput,
}

#[derive(Deserialize)]
enum PartType {
    #[serde(rename = "input_text")]
    InputText,
}

// a type name betterlite does not know keeps whatever JSON it held
#[derive(Deserialize)]
#[serde(untagged)]
enum Tag<T> {
    Known(T),
    Other(Json),
}

#[derive(Deserialize)]
struct ItemObject {
    #[serde(rename = "type", default)]
    kind: Option<Tag<ItemType>>,
    #[serde(default)]
    content: Option<Vec<ContentPart>>,
    call_id: Option<String>,
    #[serde(default, with = "serde_with::rust::double_option")]
    output: Option<Option<String>>,
}

#[derive(Deserialize)]
struct ContentPart {
    #[serde(rename = "type", default)]
    kind: Option<Tag<PartType>>,
    text: Option<String>,
}

// ast-grep-ignore: no-long-names
pub fn decode_gemini_live_client_event(
    bytes: &[u8],
    limit: usize,
) -> Result<ClientEvent, LiveClientError> {
    if bytes.len() > limit {
        return Err(LiveClientError::TooLarge { limit });
    }
    let fields: BTreeMap<Cow<'_, str>, &RawValue> = match serde_json::from_slice(bytes) {
        Ok(fields) => fields,
        Err(source) => return Err(top_level_error(source, bytes)),
    };
    match field::<Tag<KnownType>>(&fields, "type")? {
        Some(Tag::Known(KnownType::SessionUpdate)) => {
            let session = field::<Option<SessionUpdate>>(&fields, "session")?.flatten();
            Ok(ClientEvent::SessionUpdate(Box::new(
                session.unwrap_or_default(),
            )))
        }
        Some(Tag::Known(KnownType::ResponseCreate)) => Ok(ClientEvent::ResponseCreate),
        Some(Tag::Known(KnownType::ItemCreate)) => Ok(ClientEvent::ItemCreate(item_create(
            field::<ItemObject>(&fields, "item")?,
        ))),
        Some(Tag::Known(KnownType::AudioAppend)) => {
            let audio = field::<Option<String>>(&fields, "audio")?;
            Ok(ClientEvent::AudioAppend(match audio {
                None => Audio::Missing,
                Some(None) => Audio::Null,
                Some(Some(data)) => Audio::Data(data),
            }))
        }
        Some(Tag::Known(KnownType::AudioCommit)) => Ok(ClientEvent::AudioCommit),
        Some(Tag::Known(KnownType::AudioEnd)) => Ok(ClientEvent::AudioEnd),
        Some(Tag::Known(KnownType::AudioClear)) => Ok(ClientEvent::AudioClear),
        Some(Tag::Other(_)) | None => Ok(ClientEvent::Unknown),
    }
}

fn top_level_error(source: serde_json::Error, bytes: &[u8]) -> LiveClientError {
    match source.classify() {
        Category::Data => LiveClientError::NotAnObject(source),
        Category::Io | Category::Syntax | Category::Eof => {
            let message = match std::str::from_utf8(bytes) {
                Ok(text) => text.to_owned(),
                Err(error) => format!("the event is not UTF-8: {error}"),
            };
            LiveClientError::InvalidJson { message, source }
        }
    }
}

fn field<T>(
    fields: &BTreeMap<Cow<'_, str>, &RawValue>,
    name: &'static str,
) -> Result<Option<T>, LiveClientError>
where
    T: DeserializeOwned,
{
    fields
        .get(name)
        .map(|raw| {
            serde_json::from_str(raw.get())
                .map_err(|source| LiveClientError::InvalidField { path: name, source })
        })
        .transpose()
}

fn item_create(item: Option<ItemObject>) -> ItemCreate {
    let Some(object) = item else {
        return ItemCreate::UserText {
            text: String::new(),
        };
    };
    if matches!(&object.kind, Some(Tag::Known(ItemType::FunctionCallOutput))) {
        return ItemCreate::ToolResult(ToolResult {
            call_id: object.call_id,
            output: object.output,
        });
    }
    let text = user_text(&object);
    ItemCreate::UserText { text }
}

fn user_text(item: &ItemObject) -> String {
    let Some(content) = item.content.as_ref() else {
        return String::new();
    };
    let parts: Vec<&str> = content
        .iter()
        .filter(|part| matches!(&part.kind, Some(Tag::Known(PartType::InputText))))
        .filter_map(|part| part.text.as_deref())
        .filter(|text| !text.is_empty())
        .collect();
    parts.join(" ")
}
