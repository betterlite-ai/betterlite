use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::request::{Message, TokenRequest, ToolChoice};
use betterlite_tokenizer::image::settings::Settings as ImageSettings;
use betterlite_tokenizer::image::tokens::Mode;
use betterlite_tokenizer::limits::Settings as LimitSettings;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::error::TokensError;

const DEFAULT_REVISION: &str = "main";

// each wire type names itself in the error a malformed request raises, and only a type that reads this way joins
pub(crate) trait Wire: DeserializeOwned {
    const FIELD: &'static str;
}

pub(crate) fn read<T>(text: &str) -> Result<T, TokensError>
where
    T: Wire,
{
    serde_json::from_str(text).map_err(|source| TokensError::Json {
        field: T::FIELD,
        source,
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct TokenCountRequest {
    pub(crate) model: String,
    pub(crate) text: Option<String>,
    pub(crate) messages: Option<Vec<Message>>,
    pub(crate) count_response_tokens: Option<bool>,
    pub(crate) tools: Option<Vec<Json>>,
    pub(crate) tool_choice: Option<ToolChoice>,
    pub(crate) custom_tokenizer: Option<u64>,
}

impl Wire for TokenCountRequest {
    const FIELD: &'static str = "token count request";
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct EncodeRequest {
    pub(crate) model: String,
    pub(crate) text: String,
    pub(crate) custom_tokenizer: Option<u64>,
}

impl Wire for EncodeRequest {
    const FIELD: &'static str = "encode request";
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DecodeRequest {
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) tokens: Vec<u32>,
    #[serde(default)]
    pub(crate) custom_tokenizer: Option<u64>,
    // LiteLLM's decode drops special tokens unless the caller asks to keep them
    #[serde(default = "default_skip_special_tokens")]
    pub(crate) skip_special_tokens: bool,
}

const fn default_skip_special_tokens() -> bool {
    true
}

impl Wire for DecodeRequest {
    const FIELD: &'static str = "decode request";
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LoadRequest {
    Pretrained {
        identifier: String,
        #[serde(default = "default_revision")]
        revision: String,
        auth_token: Option<SecretString>,
    },
    Json {
        json: String,
    },
}

fn default_revision() -> String {
    DEFAULT_REVISION.to_owned()
}

impl Wire for LoadRequest {
    const FIELD: &'static str = "load tokenizer request";
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ImageTokensRequest {
    pub(crate) data: String,
    pub(crate) mode: Mode,
    pub(crate) base_tokens: Option<u64>,
    pub(crate) use_default_image_token_count: bool,
}

impl Wire for ImageTokensRequest {
    const FIELD: &'static str = "image tokens request";
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct TileRequest {
    pub(crate) resized_width: u32,
    pub(crate) resized_height: u32,
    pub(crate) tile_width: Option<u32>,
    pub(crate) tile_height: Option<u32>,
}

impl TileRequest {
    pub(crate) fn settings(&self) -> ImageSettings {
        let defaults = ImageSettings::default();
        ImageSettings {
            tile_width: self.tile_width.unwrap_or(defaults.tile_width),
            tile_height: self.tile_height.unwrap_or(defaults.tile_height),
            ..defaults
        }
    }
}

impl Wire for TileRequest {
    const FIELD: &'static str = "image tile request";
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct DimensionsRequest {
    pub(crate) data: String,
}

impl Wire for DimensionsRequest {
    const FIELD: &'static str = "image dimensions request";
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ResizeRequest {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl Wire for ResizeRequest {
    const FIELD: &'static str = "image resize request";
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct BoundRequest {
    pub(crate) base_tokens: Option<u64>,
}

impl Wire for BoundRequest {
    const FIELD: &'static str = "image token bound request";
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ModifiedRequest {
    pub(crate) model: String,
    pub(crate) base_model: String,
    pub(crate) messages: Option<Vec<Message>>,
    pub(crate) user_max_tokens: Option<i64>,
    pub(crate) buffer_perc: Option<f64>,
    pub(crate) buffer_num: Option<f64>,
}

impl ModifiedRequest {
    // LiteLLM counts the messages with token_counter(model=base_model, messages=messages), whose other arguments are these defaults
    pub(crate) fn input_request(&self) -> TokenRequest<'_> {
        TokenRequest {
            model: &self.base_model,
            custom_tokenizer: None,
            text: None,
            messages: self.messages.as_deref(),
            count_response_tokens: Some(false),
            tools: None,
            tool_choice: None,
        }
    }

    pub(crate) fn settings(&self) -> LimitSettings {
        let defaults = LimitSettings::default();
        LimitSettings {
            buffer_fraction: self.buffer_perc.unwrap_or(defaults.buffer_fraction),
            buffer_minimum_tokens: self.buffer_num.unwrap_or(defaults.buffer_minimum_tokens),
        }
    }
}

impl Wire for ModifiedRequest {
    const FIELD: &'static str = "modified max tokens request";
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ModelRequest {
    pub(crate) model: String,
}

impl Wire for ModelRequest {
    const FIELD: &'static str = "model request";
}

mod tests;
