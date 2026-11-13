use betterlite_config::loader::EnvironmentLookup;
use betterlite_env::EnvError;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::{Message, TokenRequest};
use indexmap::IndexMap;
use num_traits::cast::cast;
use thiserror::Error;

use super::catalog_lookup;
use super::json;
use steps::{process_messages, process_system_message};

pub mod shorten;
pub mod steps;

pub const DEFAULT_TRIM_RATIO: f64 = 0.75;
pub const DEFAULT_MAX_TRIM_ATTEMPTS: usize = 10;
const MINIMUM_AVAILABLE_TOKENS: i64 = 3;

#[derive(Debug, Error)]
pub enum TrimError {
    #[error("the message could not be read as a token counter message")]
    InvalidMessage,
    #[error("the message has no text content")]
    MissingContent,
    #[error("the messages or tools could not be read")]
    Messages(#[source] serde_json::Error),
    #[error("the token count failed")]
    Count(#[from] CountError),
    #[error("the token count does not fit")]
    CountOverflow,
    #[error("the model has no token limit in the catalog")]
    UnknownLimit,
    #[error("the model token limit does not fit")]
    LimitOverflow,
    #[error("the message was not shortened within {attempts} attempts")]
    NotShortened { attempts: usize },
    #[error("a trimming default could not be read from the environment")]
    Environment(#[from] EnvError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrimDefaults {
    pub trim_ratio: f64,
    pub max_attempts: usize,
}

impl TrimDefaults {
    pub fn load(lookup: &dyn EnvironmentLookup) -> Result<Self, TrimError> {
        let present = |name: &str| lookup.lookup(name).unwrap_or_default();
        Ok(Self {
            trim_ratio: betterlite_env::number(
                "DEFAULT_TRIM_RATIO",
                &present("DEFAULT_TRIM_RATIO"),
            )?
            .unwrap_or(DEFAULT_TRIM_RATIO),
            max_attempts: betterlite_env::number(
                "MAX_TOKEN_TRIMMING_ATTEMPTS",
                &present("MAX_TOKEN_TRIMMING_ATTEMPTS"),
            )?
            .unwrap_or(DEFAULT_MAX_TRIM_ATTEMPTS),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TrimRequest<'request> {
    pub messages: &'request [Json],
    // an empty model means the caller named none, which LiteLLM counts with its default encoding
    pub model: &'request str,
    pub trim_ratio: f64,
    pub return_response_tokens: bool,
    pub max_tokens: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimmedMessages {
    pub messages: Vec<Json>,
    pub response_tokens: Option<i64>,
}

pub struct Trimmer<'trimmer> {
    pub counter: &'trimmer Counter,
    pub catalog: &'trimmer ModelCatalog,
    pub max_attempts: usize,
}

struct TrimState {
    messages: Vec<Json>,
    max_tokens: i64,
    system_event: Option<Json>,
    tool_messages: Vec<Json>,
}

impl Trimmer<'_> {
    pub async fn trim_messages(&self, request: &TrimRequest<'_>) -> TrimmedMessages {
        match self.trimmed(request).await {
            Ok(trimmed) => trimmed,
            Err(error) => {
                // LiteLLM treats trimming as best effort and hands back the caller's list on any error
                tracing::debug!(%error, "message trimming failed");
                TrimmedMessages {
                    messages: request.messages.to_vec(),
                    response_tokens: None,
                }
            }
        }
    }

    async fn trimmed(&self, request: &TrimRequest<'_>) -> Result<TrimmedMessages, TrimError> {
        let mut messages = request.messages.to_vec();
        let Some(mut max_tokens) = self.max_tokens(request)? else {
            return Ok(TrimmedMessages {
                messages,
                response_tokens: None,
            });
        };
        let system_message = system_message_text(&messages)?;
        let tool_messages = take_tool_tail(&mut messages)?;
        let current_tokens = self.get_token_count(&messages, request.model).await?;
        if current_tokens < max_tokens {
            messages.extend(tool_messages);
            return Ok(TrimmedMessages {
                messages,
                response_tokens: None,
            });
        }
        let mut system_event = None;
        if !system_message.is_empty() {
            let (event, remaining) =
                process_system_message(self, &system_message, max_tokens, request.model).await?;
            if remaining == 0 {
                return Ok(TrimmedMessages {
                    messages: vec![event],
                    response_tokens: None,
                });
            }
            system_event = Some(event);
            max_tokens = remaining;
            messages.retain(|message| !is_system(message));
        }
        let state = TrimState {
            messages,
            max_tokens,
            system_event,
            tool_messages,
        };
        self.finish(request, &state).await
    }

    async fn finish(
        &self,
        request: &TrimRequest<'_>,
        state: &TrimState,
    ) -> Result<TrimmedMessages, TrimError> {
        let mut final_messages =
            process_messages(self, &state.messages, state.max_tokens, request.model).await?;
        if let Some(event) = state.system_event.clone() {
            final_messages.insert(0, event);
        }
        final_messages.extend(state.tool_messages.iter().cloned());
        let response_tokens = if request.return_response_tokens {
            let count = self.get_token_count(&final_messages, request.model).await?;
            Some(
                state
                    .max_tokens
                    .checked_sub(count)
                    .ok_or(TrimError::CountOverflow)?,
            )
        } else {
            None
        };
        Ok(TrimmedMessages {
            messages: final_messages,
            response_tokens,
        })
    }

    fn max_tokens(&self, request: &TrimRequest<'_>) -> Result<Option<i64>, TrimError> {
        if let Some(max_tokens) = request.max_tokens {
            return Ok(Some(max_tokens));
        }
        let Some(entry) = catalog_lookup::exact(self.catalog, request.model) else {
            return Ok(None);
        };
        // LiteLLM reads max_tokens as the default of .get("max_input_tokens"), so a missing max_tokens raises there
        let metadata = &entry.fields.metadata;
        let fallback = metadata.max_tokens.ok_or(TrimError::UnknownLimit)?;
        let model_limit = metadata.max_input_tokens.unwrap_or(fallback);
        let limit = cast::<u64, f64>(model_limit).ok_or(TrimError::LimitOverflow)?;
        Ok(Some(
            cast::<f64, i64>(limit.mul_add(request.trim_ratio, 0.0_f64))
                .ok_or(TrimError::LimitOverflow)?,
        ))
    }

    pub async fn get_token_count(&self, messages: &[Json], model: &str) -> Result<i64, TrimError> {
        let value = serde_json::to_value(messages).map_err(TrimError::Messages)?;
        let parsed: Vec<Message> = serde_json::from_value(value).map_err(TrimError::Messages)?;
        let request = TokenRequest {
            model,
            custom_tokenizer: None,
            text: None,
            messages: Some(&parsed),
            count_response_tokens: Some(false),
            tools: None,
            tool_choice: None,
        };
        let count = self.counter.count(&request).await?;
        i64::try_from(count).map_err(|_conversion| TrimError::CountOverflow)
    }
}

fn system_message_event(system_message: &str) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(String::from("role"), Json::Text(String::from("system")));
    fields.insert(
        String::from("content"),
        Json::Text(system_message.to_owned()),
    );
    Json::Object(fields)
}

fn role_of(message: &Json) -> Result<Option<&str>, TrimError> {
    let fields = json::object(message).ok_or(TrimError::InvalidMessage)?;
    fields.get("role").map_or_else(
        || Err(TrimError::InvalidMessage),
        |role| Ok(json::text(role)),
    )
}

fn system_message_text(messages: &[Json]) -> Result<String, TrimError> {
    let mut system_message = String::new();
    for message in messages {
        if role_of(message)? != Some("system") {
            continue;
        }
        let content = json::object(message)
            .and_then(|fields| fields.get("content"))
            .and_then(json::text)
            .ok_or(TrimError::MissingContent)?;
        if !system_message.is_empty() {
            system_message.push('\n');
        }
        system_message.push_str(content);
    }
    Ok(system_message)
}

fn take_tool_tail(messages: &mut Vec<Json>) -> Result<Vec<Json>, TrimError> {
    let mut tool_messages = Vec::new();
    for message in messages.iter().rev() {
        if role_of(message)? != Some("tool") {
            break;
        }
        tool_messages.push(message.clone());
    }
    tool_messages.reverse();
    messages.truncate(messages.len().saturating_sub(tool_messages.len()));
    Ok(tool_messages)
}

fn is_system(message: &Json) -> bool {
    json::object(message)
        .and_then(|fields| fields.get("role"))
        .and_then(json::text)
        == Some("system")
}
