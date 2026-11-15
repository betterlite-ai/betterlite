#![cfg(test)]

use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_config::sections::vertex_schema_shaping::Settings as SchemaSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use futures_util::StreamExt as _;

use crate::gemini_options::defaults::OptionSettings;
use crate::gemini_thinking::settings::{MinimalEffortSupport, ThinkingBudgets, ThinkingSettings};
use crate::model_policy::capabilities::{
    CapabilityKind, CapabilityLookupError, ModelCapabilityLookup,
};
use crate::vertex_batch_upload::rows::{RowContext, RowMappers};
use crate::vertex_batch_upload::sources::FileTypes;
use crate::vertex_batch_upload::upload::error::UploadError;
use crate::vertex_batch_upload::upload::vertex_batch_upload;

pub(crate) const MAX_LINE_BYTES: usize = 0x0010_0000;

pub(crate) struct Support;

impl ModelCapabilityLookup for Support {
    fn supports(
        &self,
        _kind: CapabilityKind,
        _model: &str,
        _provider: &str,
    ) -> Result<bool, CapabilityLookupError> {
        Ok(true)
    }
}

impl MinimalEffortSupport for Support {
    fn disables_minimal(&self, _model: &str) -> bool {
        false
    }
}

pub(crate) struct NoFetch;

impl RemoteMediaFetcher for NoFetch {
    fn convert_url(&self, url: &str) -> impl Future<Output = Result<String, ImageFetchError>> {
        ready(Err(ImageFetchError::Attempts {
            url: url.to_owned(),
        }))
    }

    fn convert_urls(
        &self,
        urls: &[String],
    ) -> impl Future<Output = Result<Vec<String>, ImageFetchError>> {
        let url = urls.first().cloned().unwrap_or_default();
        ready(Err(ImageFetchError::Attempts { url }))
    }
}

// the injected bridges: `input` text becomes one user message, and a developer message keeps only role and content
pub(crate) struct Mappers;

impl RowMappers for Mappers {
    type Error = Infallible;

    fn responses_to_chat(&self, _provider: &str, body: &Object) -> Result<Object, Infallible> {
        let mut chat = Object::new();
        if let Some(model) = body.get("model") {
            chat.insert("model".to_owned(), model.clone());
        }
        let content = body.get("input").cloned().unwrap_or(Json::Null);
        let message = Object::from([
            ("role".to_owned(), Json::Text("user".to_owned())),
            ("content".to_owned(), content),
        ]);
        chat.insert(
            "messages".to_owned(),
            Json::List(vec![Json::Object(message)]),
        );
        Ok(chat)
    }

    fn developer_to_system(&self, messages: Vec<Object>) -> Result<Vec<Object>, Infallible> {
        Ok(messages
            .into_iter()
            .map(|message| {
                if message.get("role") != Some(&Json::Text("developer".to_owned())) {
                    return message;
                }
                let content = message.get("content").cloned().unwrap_or(Json::Null);
                Object::from([
                    ("role".to_owned(), Json::Text("system".to_owned())),
                    ("content".to_owned(), content),
                ])
            })
            .collect())
    }
}

pub(crate) struct Settings {
    pub(crate) options: OptionSettings,
    pub(crate) schema: SchemaSettings,
    pub(crate) thinking: ThinkingSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            options: OptionSettings::default(),
            schema: SchemaSettings::default(),
            thinking: ThinkingSettings {
                budgets: ThinkingBudgets {
                    minimal: 128,
                    minimal_flash: 1,
                    minimal_pro: 128,
                    minimal_flash_lite: 512,
                    low: 1024,
                    medium: 2048,
                    high: 4096,
                    disable: 0,
                },
                default_thinking_level_low: false,
            },
        }
    }
}

pub(crate) fn context(settings: &Settings) -> RowContext<'_, Mappers, NoFetch> {
    RowContext {
        mappers: &Mappers,
        fetcher: &NoFetch,
        capabilities: &Support,
        options: &settings.options,
        schema: &settings.schema,
        thinking: &settings.thinking,
        support: &Support,
        modify_params: false,
    }
}

// pulls at most `count` chunks, so a test sees what the stream does after exactly that many polls
pub(crate) async fn upload(
    file: FileTypes,
    purpose: &str,
    passthrough: &Json,
    count: usize,
) -> Vec<Result<Vec<u8>, UploadError<Infallible>>> {
    let settings = Settings::default();
    let context = context(&settings);
    vertex_batch_upload(file, purpose, passthrough, &context, MAX_LINE_BYTES)
        .take(count)
        .collect()
        .await
}
