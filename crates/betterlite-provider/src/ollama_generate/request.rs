use std::collections::BTreeMap;
use std::io::Cursor;

use base64::Engine as _;
use base64::alphabet::STANDARD as BASE64_ALPHABET;
use base64::engine::GeneralPurpose;
use base64::engine::GeneralPurposeConfig;
use base64::engine::general_purpose::STANDARD;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::builtin::Message as BuiltinMessage;
use betterlite_prompts::builtin::images::convert_to_ollama_image;
use betterlite_prompts::builtin::ollama::ollama;
use betterlite_prompts::factory::custom::{CustomPromptTemplate, custom_prompt};
use betterlite_prompts::media::inline::{RemoteMediaFetcher, async_inline_remote_media};
use betterlite_prompts::media::parts::INLINE_REMOTE_IMAGE_URLS;
use betterlite_prompts::message_content::{Content, ContentPart, Message, get_str_from_messages};
use image::DynamicImage;
use image::ImageReader;
use image::Limits;
use image::codecs::jpeg::JpegEncoder;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;

use super::error::OllamaGenerateError;
use crate::ollama_chat::error::OllamaError;

const FORMAT: &str = "format";
const IMAGES: &str = "images";
const STREAM: &str = "stream";
const THINK: &str = "think";
const DATA_PREFIX: &str = "data:";
const JPEG_SIGNATURE: [u8; 3] = [0xFF, 0xD8, 0xFF];
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
// Pillow saves without a quality argument at its 75 default
const JPEG_QUALITY: u8 = 75;
// a 1 MiB prompt cap can still carry a header that declares huge dimensions, so the decode is bounded
const MAX_IMAGE_SIDE: u32 = 8192;
const MAX_IMAGE_ALLOC: u64 = 0x0400_0000;
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &BASE64_ALPHABET,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RequestParams {
    #[serde(default)]
    pub custom_prompt_dict: Option<BTreeMap<String, CustomPromptTemplate>>,
    #[serde(default)]
    pub text_completion: bool,
}

pub struct RequestInput<'request> {
    pub model: &'request str,
    pub messages: &'request [Message],
    pub optional_params: &'request IndexMap<String, Json>,
    pub params: &'request RequestParams,
    pub custom_prompts: &'request BTreeMap<String, CustomPromptTemplate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OllamaGenerateRequest {
    pub model: String,
    pub prompt: String,
    pub options: IndexMap<String, Json>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<Json>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub think: Option<Json>,
}

// the builtin renderers read the builtin message shape, so the wire message goes through its JSON form
#[derive(Serialize)]
struct RendererMessage<'request> {
    role: &'request str,
    content: RendererContent<'request>,
    #[serde(flatten)]
    fields: &'request Object,
}

#[derive(Serialize)]
#[serde(untagged)]
enum RendererContent<'request> {
    Text(&'request str),
    Parts(&'request [ContentPart]),
    Null,
}

pub fn transform_request(
    input: &RequestInput<'_>,
) -> Result<OllamaGenerateRequest, OllamaGenerateError> {
    let mut optional = input.optional_params.clone();
    let prompts = input
        .params
        .custom_prompt_dict
        .as_ref()
        .unwrap_or(input.custom_prompts);
    let prompt = match prompts.get(input.model) {
        Some(template) => custom_prompt(template, &renderer_messages(input.messages)?)?,
        None if input.params.text_completion => get_str_from_messages(input.messages),
        None => {
            let rendered = ollama(input.model, &renderer_messages(input.messages)?)?;
            let images = rendered.images.into_iter().map(Json::Text).collect();
            optional.insert(String::from(IMAGES), Json::List(images));
            rendered.prompt
        }
    };
    let stream = match optional.shift_remove(STREAM) {
        Some(Json::Bool(value)) => value,
        Some(Json::Null) | None => false,
        Some(_) => {
            return Err(OllamaGenerateError::from(OllamaError::Param {
                field: STREAM,
            }));
        }
    };
    let format = take_nullable(&mut optional, FORMAT);
    let images = take_images(&mut optional)?;
    let think = take_nullable(&mut optional, THINK);
    Ok(OllamaGenerateRequest {
        model: input.model.to_owned(),
        prompt,
        options: optional,
        stream,
        format,
        images,
        think,
    })
}

pub async fn async_transform_request<Fetcher>(
    input: &RequestInput<'_>,
    fetcher: &Fetcher,
) -> Result<OllamaGenerateRequest, OllamaGenerateError>
where
    Fetcher: RemoteMediaFetcher + Sync,
{
    let messages =
        async_inline_remote_media(input.messages, &INLINE_REMOTE_IMAGE_URLS, fetcher).await?;
    transform_request(&RequestInput {
        model: input.model,
        messages: &messages,
        optional_params: input.optional_params,
        params: input.params,
        custom_prompts: input.custom_prompts,
    })
}

fn renderer_messages(messages: &[Message]) -> Result<Vec<BuiltinMessage>, OllamaGenerateError> {
    let mut rendered = Vec::with_capacity(messages.len());
    for message in messages {
        let content = match &message.content {
            Some(Content::Text(text)) => RendererContent::Text(text),
            Some(Content::Parts(parts)) => RendererContent::Parts(parts),
            None => RendererContent::Null,
        };
        let prepared = RendererMessage {
            role: &message.role,
            content,
            fields: &message.provider_specific_fields,
        };
        let bytes = serde_json::to_vec(&prepared)
            .map_err(|source| OllamaGenerateError::Message { source })?;
        rendered.push(
            serde_json::from_slice::<BuiltinMessage>(&bytes)
                .map_err(|source| OllamaGenerateError::Message { source })?,
        );
    }
    Ok(rendered)
}

fn take_nullable(optional: &mut IndexMap<String, Json>, name: &str) -> Option<Json> {
    match optional.shift_remove(name) {
        Some(Json::Null) | None => None,
        Some(value) => Some(value),
    }
}

fn take_images(
    optional: &mut IndexMap<String, Json>,
) -> Result<Option<Vec<String>>, OllamaGenerateError> {
    let Some(value) = optional.shift_remove(IMAGES) else {
        return Ok(None);
    };
    let items = match value {
        Json::List(items) => items,
        Json::Null => return Ok(None),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => {
            return Err(OllamaGenerateError::from(OllamaError::Param {
                field: IMAGES,
            }));
        }
    };
    let mut images = Vec::with_capacity(items.len());
    for item in items {
        let Json::Text(image) = item else {
            return Err(OllamaGenerateError::from(OllamaError::Param {
                field: IMAGES,
            }));
        };
        images.push(convert_image(&convert_to_ollama_image(&image)?)?);
    }
    Ok(Some(images))
}

// LiteLLM sends JPEG and PNG bytes as they are and re-encodes every other format as JPEG
pub fn convert_image(image: &str) -> Result<String, OllamaGenerateError> {
    let payload = match image.strip_prefix(DATA_PREFIX) {
        Some(_) => image.rsplit(',').next().unwrap_or(image),
        None => image,
    };
    let Some(raw) = decode_base64(payload) else {
        return Ok(image.to_owned());
    };
    if raw.starts_with(&JPEG_SIGNATURE) || raw.starts_with(&PNG_SIGNATURE) {
        return Ok(payload.to_owned());
    }
    reencode_as_jpeg(&raw, image)
}

fn reencode_as_jpeg(raw: &[u8], original: &str) -> Result<String, OllamaGenerateError> {
    let raw_reader = ImageReader::new(Cursor::new(raw));
    let Ok(mut reader) = raw_reader.with_guessed_format() else {
        return Ok(original.to_owned());
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_IMAGE_ALLOC);
    reader.limits(limits);
    let Ok(decoded) = reader.decode() else {
        return Ok(original.to_owned());
    };
    let rgb = DynamicImage::ImageRgb8(decoded.to_rgb8());
    let mut jpeg = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY);
    encoder
        .encode_image(&rgb)
        .map_err(|source| OllamaGenerateError::ImageEncode { source })?;
    Ok(STANDARD.encode(jpeg))
}

// Python's non-strict base64 drops every character outside the alphabet, counts '=' and pads what is missing
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let mut alphabet = String::with_capacity(text.len());
    let mut padding = 0_usize;
    for character in text.chars() {
        match character {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => alphabet.push(character),
            '=' => padding = padding.saturating_add(1),
            other if other.is_ascii() => {}
            _ => return None,
        }
    }
    let remainder = alphabet.len().checked_rem(4)?;
    if remainder == 1_usize {
        return None;
    }
    let needed = 4_usize
        .checked_sub(remainder)
        .and_then(|left| left.checked_rem(4))?;
    if padding < needed {
        return None;
    }
    for _ in 0..needed {
        alphabet.push('=');
    }
    LENIENT.decode(alphabet).ok()
}
