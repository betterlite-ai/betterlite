use std::borrow::Cow;
use std::collections::BTreeMap;

use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::error::Category;
use serde_json::value::RawValue;

use super::error::InputError;
use crate::realtime_session::records::Setting;
use crate::realtime_session::update::SessionUpdate;

#[derive(Debug)]
pub enum ClientEvent {
    SessionUpdate(Box<SessionUpdate>),
    AudioAppend(Setting<String>),
    AudioCommit,
    ItemCreate(ItemCreate),
    ResponseCreate,
    ResponseCancel,
    Unknown(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ItemCreate {
    Message { texts: Vec<String> },
    ToolResult { call_id: String, content: String },
    Ignored,
}

#[derive(Deserialize)]
enum KnownType {
    #[serde(rename = "session.update")]
    SessionUpdate,
    #[serde(rename = "input_audio_buffer.append")]
    AudioAppend,
    #[serde(rename = "input_audio_buffer.commit")]
    AudioCommit,
    #[serde(rename = "conversation.item.create")]
    ItemCreate,
    #[serde(rename = "response.create")]
    ResponseCreate,
    #[serde(rename = "response.cancel")]
    ResponseCancel,
}

#[derive(Deserialize)]
enum ItemType {
    #[serde(rename = "message")]
    Message,
    #[serde(rename = "function_call_output")]
    FunctionCallOutput,
}

#[derive(Deserialize)]
enum PartType {
    #[serde(rename = "input_text")]
    InputText,
}

// A type name betterlite does not know still has to reach the warning, so it is kept as JSON
#[derive(Deserialize)]
#[serde(untagged)]
enum Tag<T> {
    Known(T),
    Other(Json),
}

#[derive(Deserialize)]
struct ItemObject {
    #[serde(rename = "type")]
    kind: Option<Tag<ItemType>>,
    #[serde(default)]
    content: Vec<ContentPart>,
    call_id: Option<String>,
    #[serde(default = "default_output")]
    output: Json,
}

#[derive(Deserialize)]
struct ContentPart {
    #[serde(rename = "type")]
    kind: Option<Tag<PartType>>,
    text: Option<String>,
}

const fn default_output() -> Json {
    Json::Text(String::new())
}

fn shape(source: serde_json::Error) -> InputError {
    match source.classify() {
        Category::Data => InputError::Malformed { source },
        Category::Io | Category::Syntax | Category::Eof => InputError::InvalidJson { source },
    }
}

fn field<T>(fields: &BTreeMap<Cow<'_, str>, &RawValue>, name: &str) -> Result<Option<T>, InputError>
where
    T: DeserializeOwned,
{
    fields
        .get(name)
        .map(|raw| serde_json::from_str(raw.get()).map_err(shape))
        .transpose()
}

pub fn decode_realtime_client_input(bytes: &[u8]) -> Result<ClientEvent, InputError> {
    let fields: BTreeMap<Cow<'_, str>, &RawValue> = serde_json::from_slice(bytes).map_err(shape)?;
    let kind = field::<Tag<KnownType>>(&fields, "type")?;
    Ok(match kind {
        Some(Tag::Known(KnownType::SessionUpdate)) => {
            let session = field(&fields, "session")?.unwrap_or_default();
            ClientEvent::SessionUpdate(Box::new(SessionUpdate { session }))
        }
        Some(Tag::Known(KnownType::AudioAppend)) => {
            let audio = field(&fields, "audio")?;
            ClientEvent::AudioAppend(audio.unwrap_or_else(|| Setting(Some(String::new()))))
        }
        Some(Tag::Known(KnownType::AudioCommit)) => ClientEvent::AudioCommit,
        Some(Tag::Known(KnownType::ItemCreate)) => {
            ClientEvent::ItemCreate(item_create(field(&fields, "item")?))
        }
        Some(Tag::Known(KnownType::ResponseCreate)) => ClientEvent::ResponseCreate,
        Some(Tag::Known(KnownType::ResponseCancel)) => ClientEvent::ResponseCancel,
        Some(Tag::Other(Json::Text(name))) => ClientEvent::Unknown(name),
        // A null type is the same as no type at all
        Some(Tag::Other(Json::Null)) | None => ClientEvent::Unknown("None".to_owned()),
        Some(Tag::Other(other)) => ClientEvent::Unknown(dumps(&other)),
    })
}

fn item_create(item: Option<ItemObject>) -> ItemCreate {
    let Some(object) = item else {
        return ItemCreate::Ignored;
    };
    match object.kind {
        Some(Tag::Known(ItemType::Message)) => ItemCreate::Message {
            texts: object
                .content
                .into_iter()
                .filter(|part| matches!(part.kind, Some(Tag::Known(PartType::InputText))))
                .map(|part| part.text.unwrap_or_default())
                .collect(),
        },
        Some(Tag::Known(ItemType::FunctionCallOutput)) => ItemCreate::ToolResult {
            call_id: object.call_id.unwrap_or_default(),
            content: match object.output {
                Json::Text(text) => text,
                other @ (Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::List(_)
                | Json::Object(_)) => dumps(&other),
            },
        },
        Some(Tag::Other(_)) | None => ItemCreate::Ignored,
    }
}
