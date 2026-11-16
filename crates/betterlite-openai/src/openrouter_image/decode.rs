use crate::api::image_contracts::records::ImageData;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::de::value::MapAccessDeserializer;
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::de::Deserializer as JsonDeserializer;
use serde_json::error::Category;
use serde_with::rust::double_option;
use std::fmt;
use std::marker::PhantomData;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum ReplyDecodeError {
    #[error("{source}")]
    Json { source: serde_json::Error },
    #[error("{source}")]
    Shape {
        source: serde_path_to_error::Error<serde_json::Error>,
    },
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenRouterImageUsage {
    pub prompt_tokens: i64,
    pub total_tokens: i64,
    pub output_image_tokens: i64,
    pub input_image_tokens: i64,
    pub cost: Option<f64>,
    pub cost_details: IndexMap<String, Json>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenRouterImageReply {
    pub images: Vec<ImageData>,
    pub usage: Option<OpenRouterImageUsage>,
    pub model: Option<Json>,
}
// serde derives also read a list as a struct, and the source's .get() fails on a list
#[derive(Default)]
struct Object<T> {
    fields: T,
    keys: usize,
}
struct ObjectVisitor<T>(PhantomData<T>);
struct KeyCount<'count, A> {
    map: A,
    keys: &'count mut usize,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Reply {
    choices: Vec<Object<Choice>>,
    usage: Option<Object<Usage>>,
    #[serde(with = "double_option")]
    model: Option<Option<String>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Choice {
    message: Object<Message>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Message {
    images: Vec<Object<Entry>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Entry {
    image_url: Object<ImageUrl>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct ImageUrl {
    url: Option<String>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Usage {
    prompt_tokens: Option<i64>,
    total_tokens: Option<i64>,
    completion_tokens_details: Option<Object<TokenDetails>>,
    prompt_tokens_details: Option<Object<TokenDetails>>,
    cost: Option<f64>,
    cost_details: Option<IndexMap<String, Json>>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct TokenDetails {
    image_tokens: Option<i64>,
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ObjectVisitor(PhantomData))
    }
}
impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
    type Value = Object<T>;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        let mut keys = 0_usize;
        let fields = T::deserialize(MapAccessDeserializer::new(KeyCount {
            map,
            keys: &mut keys,
        }))?;
        Ok(Object { fields, keys })
    }
}
impl<'de, A: MapAccess<'de>> MapAccess<'de> for KeyCount<'_, A> {
    type Error = A::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        let key = self.map.next_key_seed(seed)?;
        if key.is_some() {
            *self.keys = self.keys.saturating_add(1);
        }
        Ok(key)
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.map.next_value_seed(seed)
    }
}
pub fn decode_openrouter_image_reply(
    bytes: &[u8],
) -> Result<OpenRouterImageReply, ReplyDecodeError> {
    // the source parses the whole body before it reads a field, so a syntax error beats a shape error
    serde_json::from_slice::<IgnoredAny>(bytes)
        .map_err(|source| ReplyDecodeError::Json { source })?;
    let reply: Object<Reply> = serde_path_to_error::deserialize(&mut JsonDeserializer::from_slice(
        bytes,
    ))
    .map_err(|error| {
        if error.inner().classify() == Category::Data {
            ReplyDecodeError::Shape { source: error }
        } else {
            ReplyDecodeError::Json {
                source: error.into_inner(),
            }
        }
    })?;
    let mut images = Vec::new();
    for choice in reply.fields.choices {
        for entry in choice.fields.message.fields.images {
            let url = entry.fields.image_url.fields.url;
            if let Some(link) = url.filter(|link| !link.is_empty()) {
                images.push(image_data(&link));
            }
        }
    }
    Ok(OpenRouterImageReply {
        images,
        usage: reply
            .fields
            .usage
            .filter(|usage| usage.keys > 0)
            .map(|usage| usage_record(usage.fields)),
        model: reply
            .fields
            .model
            .map(|model| model.map_or(Json::Null, Json::Text)),
    })
}
fn usage_record(usage: Usage) -> OpenRouterImageUsage {
    OpenRouterImageUsage {
        prompt_tokens: usage.prompt_tokens.unwrap_or_default(),
        total_tokens: usage.total_tokens.unwrap_or_default(),
        output_image_tokens: details_image_tokens(usage.completion_tokens_details),
        input_image_tokens: details_image_tokens(usage.prompt_tokens_details),
        cost: usage.cost,
        cost_details: usage.cost_details.unwrap_or_default(),
    }
}
fn details_image_tokens(details: Option<Object<TokenDetails>>) -> i64 {
    details
        .and_then(|counted| counted.fields.image_tokens)
        .unwrap_or_default()
}
fn image_data(url: &str) -> ImageData {
    match url.strip_prefix("data:") {
        Some(data) => ImageData {
            b64_json: data.split_once(',').map(|(_, encoded)| encoded.to_owned()),
            url: None,
            revised_prompt: None,
        },
        None => ImageData {
            b64_json: None,
            url: Some(url.to_owned()),
            revised_prompt: None,
        },
    }
}
