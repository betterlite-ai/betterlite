use betterlite_config::loader::EnvironmentLookup;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::media_fetch::error::ImageFetchError;
use betterlite_prompts::message_content::Message;
use betterlite_prompts::message_fields::image_hoist::hoist_tool_message_images;
use betterlite_prompts::message_fields::system_runs::system_messages_first;
use betterlite_prompts::message_fields::tool_reference::drop_tool_reference_parts;
use thiserror::Error;

use crate::chat::ChatMessage;

mod cache_control;
mod messages;
mod tests;
mod tools;

const OPENAI: &str = "openai";
// LiteLLM's OPENAI_SYSTEM_MESSAGES_FIRST_PROVIDERS
const INSTRUCTION_FIRST_PROVIDERS: [&str; 2] = ["azure", "openai"];

#[derive(Debug, Error)]
pub enum ChatContentError {
    #[error("content block has type='file' but is missing the required 'file' field")]
    MissingFile(MissingFileError),
    #[error(transparent)]
    Fetch(#[from] ImageFetchError),
    #[error("a chat message could not be converted for the prompt helpers: {source}")]
    MessageShape {
        #[source]
        source: serde_json::Error,
    },
}

impl ChatContentError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::MissingFile(_) | Self::Fetch(_) => 400,
            Self::MessageShape { .. } => 500,
        }
    }
}

// LiteLLM raises the missing-file BadRequestError for the openai provider with no model named
#[derive(Debug)]
pub struct MissingFileError {
    pub provider: &'static str,
    pub model: Option<String>,
}

// The failed-download seam B5083's Fetcher fills; this module never opens a request path or a local file
pub trait UrlDownloader {
    fn download(&self, url: &str) -> Result<String, ImageFetchError>;
}

// The open URL-download decision: a file_id that needs bytes fails as LiteLLM does with downloads off
pub struct RefuseDownloads;

impl UrlDownloader for RefuseDownloads {
    fn download(&self, url: &str) -> Result<String, ImageFetchError> {
        Err(ImageFetchError::Disabled {
            url: url.to_owned(),
        })
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Sync,
    AsyncBaseClass,
    AsyncSubclass,
}

pub struct ChatContent<'request> {
    pub model: &'request str,
    pub messages: &'request [ChatMessage],
    pub optional_params: &'request Object,
    pub provider: Option<&'request str>,
    pub api_base: Option<&'request str>,
    pub global_api_base: Option<&'request str>,
    pub openai_system_messages_first: bool,
    pub environment: &'request dyn EnvironmentLookup,
    pub downloads: &'request dyn UrlDownloader,
}

#[derive(Debug)]
pub struct TransformedRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub optional_params: Object,
}

impl TransformedRequest {
    pub fn to_body(&self) -> Result<Object, ChatContentError> {
        let mut body = Object::new();
        body.insert("model".to_owned(), Json::Text(self.model.clone()));
        body.insert(
            "messages".to_owned(),
            Json::List(messages::as_json(&self.messages)?),
        );
        for (name, value) in &self.optional_params {
            body.insert(name.clone(), value.clone());
        }
        Ok(body)
    }
}

impl ChatContent<'_> {
    pub fn transform(&self, mode: Mode) -> Result<TransformedRequest, ChatContentError> {
        let mut messages = self.transformed_messages()?;
        let mut optional_params = self.optional_params.clone();
        let hosted = self.targets_openai_hosted_endpoint(self.api_base);
        if self.provider != Some(OPENAI) || hosted {
            for message in &mut messages {
                cache_control::strip_message(message);
            }
            cache_control::strip_tools(&mut optional_params);
        }
        // The async base class keeps max_retries, and a subclass falls through to transform_request
        if !matches!(mode, Mode::AsyncBaseClass) {
            optional_params.shift_remove("max_retries");
        }
        let has_tools = optional_params.get("tools").is_some_and(Json::is_truthy)
            || optional_params
                .get("functions")
                .is_some_and(Json::is_truthy);
        if !has_tools {
            optional_params.shift_remove("tool_choice");
        }
        if let Some(tools) = tools::sanitized_tools(&optional_params, self.provider, hosted) {
            optional_params.insert("tools".to_owned(), Json::List(tools));
        }
        let typed_messages = messages
            .iter()
            .map(messages::chat_message)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TransformedRequest {
            model: self.model.to_owned(),
            messages: typed_messages,
            optional_params,
        })
    }

    fn transformed_messages(&self) -> Result<Vec<Message>, ChatContentError> {
        let prompt_messages = self
            .messages
            .iter()
            .map(messages::prompt_message)
            .collect::<Result<Vec<_>, _>>()?;
        let ordered = match self.provider {
            Some(provider)
                if self.openai_system_messages_first
                    && INSTRUCTION_FIRST_PROVIDERS.contains(&provider) =>
            {
                system_messages_first(&prompt_messages)
            }
            _ => prompt_messages,
        };
        let stripped = drop_tool_reference_parts(&ordered);
        let hoisted = hoist_tool_message_images(&stripped);
        let mut transformed = Vec::with_capacity(hoisted.len());
        for message in hoisted {
            transformed.push(messages::transform_user_content(message, self.downloads)?);
        }
        Ok(transformed)
    }

    fn targets_openai_hosted_endpoint(&self, api_base: Option<&str>) -> bool {
        if self.provider != Some(OPENAI) {
            return false;
        }
        let resolved = api_base
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                self.global_api_base
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            })
            .or_else(|| {
                self.environment
                    .lookup("OPENAI_BASE_URL")
                    .filter(|value| !value.is_empty())
            })
            .or_else(|| {
                self.environment
                    .lookup("OPENAI_API_BASE")
                    .filter(|value| !value.is_empty())
            });
        let Some(base) = resolved else {
            return true;
        };
        let Ok(parsed) = url::Url::parse(&base) else {
            return true;
        };
        let Some(hostname) = parsed.host_str() else {
            return true;
        };
        hostname == "openai.com" || hostname.ends_with(".openai.com")
    }
}
