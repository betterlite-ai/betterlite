use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use serde::Deserialize;

pub(crate) type Environment = BTreeMap<String, String>;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    ToDataUrl {
        sources: Vec<Source>,
    },
    Prepare {
        model: String,
        prompt: Option<String>,
        #[serde(default)]
        images: Vec<Option<PrepareImage>>,
        #[serde(default)]
        mask: Vec<Option<PrepareImage>>,
        #[serde(default)]
        params: ImageParams,
    },
    Map {
        model: String,
        options: Vec<ImageParams>,
    },
    Url {
        environment: Environment,
        api_bases: Vec<Option<String>>,
        models: Vec<String>,
    },
    Reply {
        replies: Vec<ReplyCase>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Source {
    Bytes { content: Vec<u8> },
    Path { name: String, content: Vec<u8> },
    Reader { content: Vec<u8>, position: u64 },
    Tuple { content: Vec<u8> },
    Value { value: Json },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PrepareImage {
    Bytes { content: Vec<u8> },
    Path { name: String, content: Vec<u8> },
    Reader { content: Vec<u8>, position: u64 },
    Tuple { content: Vec<u8> },
    Text { text: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyCase {
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: ImageHeaders,
    pub(crate) body: String,
}
