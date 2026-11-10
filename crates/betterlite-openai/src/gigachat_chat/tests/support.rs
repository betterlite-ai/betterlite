#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::chat::ChatMessage;
use crate::gigachat_chat::config::{GigachatBearer, GigachatChatConfig, GigachatImageUploader};
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_utils::IdSource;
use crate::openai_base_config::{Headers, Object};

pub(super) const FIXED: u128 = 0x0123_4567_89ab_4def_8123_4567_89ab_cdef;

pub(super) struct FixedIds;

impl IdSource for FixedIds {
    fn new_uuid(&self) -> Uuid {
        Uuid::from_u128(FIXED)
    }
}

pub(super) struct ScriptedBearer {
    reply: Result<SecretString, u16>,
    seen: Mutex<Vec<Option<String>>>,
}

impl ScriptedBearer {
    fn issuing(token: &str) -> Self {
        Self {
            reply: Ok(SecretString::from(token)),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn failing(status_code: u16) -> Self {
        Self {
            reply: Err(status_code),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn credentials(&self) -> Vec<Option<String>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl GigachatBearer for ScriptedBearer {
    async fn access_token(
        &self,
        credentials: Option<&SecretString>,
    ) -> Result<SecretString, GigaChatError> {
        self.seen
            .lock()
            .unwrap()
            .push(credentials.map(|value| value.expose_secret().to_owned()));
        self.reply.clone().map_err(|status_code| GigaChatError {
            status_code,
            message: String::from("the token could not be fetched"),
            headers: None,
            source: None,
        })
    }
}

#[derive(Default)]
pub(super) struct ScriptedUploader {
    ids: BTreeMap<String, String>,
    seen: Mutex<Vec<String>>,
}

impl ScriptedUploader {
    pub(super) fn with(uploads: &[(&str, &str)]) -> Self {
        Self {
            ids: uploads
                .iter()
                .map(|(url, id)| ((*url).to_owned(), (*id).to_owned()))
                .collect(),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn uploads(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl GigachatImageUploader for ScriptedUploader {
    async fn upload(&self, image_url: &str) -> Option<String> {
        self.seen.lock().unwrap().push(image_url.to_owned());
        self.ids.get(image_url).cloned()
    }
}

pub(super) struct Fixture {
    pub(super) bearer: ScriptedBearer,
    pub(super) uploader: ScriptedUploader,
    pub(super) ids: FixedIds,
}

impl Fixture {
    pub(super) fn issuing(token: &str, uploads: &[(&str, &str)]) -> Self {
        Self {
            bearer: ScriptedBearer::issuing(token),
            uploader: ScriptedUploader::with(uploads),
            ids: FixedIds,
        }
    }

    pub(super) fn failing(status_code: u16) -> Self {
        Self {
            bearer: ScriptedBearer::failing(status_code),
            uploader: ScriptedUploader::default(),
            ids: FixedIds,
        }
    }

    #[must_use]
    pub(super) fn config(&self) -> GigachatChatConfig<'_> {
        GigachatChatConfig {
            bearer: &self.bearer,
            uploader: &self.uploader,
            ids: &self.ids,
        }
    }
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

pub(super) fn messages(text: &str) -> Vec<ChatMessage> {
    serde_json::from_str(text).unwrap()
}

pub(super) fn headers(text: &str) -> Headers {
    serde_json::from_str(text).unwrap()
}

pub(super) fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}
