use std::mem::take;

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::gemini_tool_results::convert::convert_tool_result_to_gemini;
use betterlite_prompts::gemini_tool_results::types::{GeminiToolResult, LastToolCallMessage};
use betterlite_prompts::media::inline::RemoteMediaFetcher;
use betterlite_prompts::media::limits::MAX_MESSAGES;
use betterlite_prompts::message_content::Message;

use crate::gemini_history::content::{Content, Role, object, object_of};
use crate::gemini_history::error::HistoryError;
use crate::gemini_history::turns::{AssistantOptions, append_assistant_parts, append_user_parts};
use crate::model_urls::model_name::is_gemini_3_or_newer;

const USER: &str = "user";
const SYSTEM: &str = "system";
const ASSISTANT: &str = "assistant";
const TOOL: &str = "tool";
const FUNCTION: &str = "function";
const SPACE: &str = " ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryWarning {
    BlankText,
    EmptyHistory,
}

impl HistoryWarning {
    #[must_use]
    pub const fn subject(self) -> &'static str {
        match self {
            Self::BlankText => "blank_text",
            Self::EmptyHistory => "empty_history",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub contents: Vec<Content>,
    pub warnings: Vec<HistoryWarning>,
}

pub async fn convert_messages_with_history<Source>(
    messages: &[Message],
    model: Option<&str>,
    fetcher: &Source,
) -> Result<History, HistoryError>
where
    Source: RemoteMediaFetcher + Sync,
{
    if messages.len() > MAX_MESSAGES {
        return Err(HistoryError::TooManyMessages(MAX_MESSAGES));
    }
    let mut walker = Walker {
        messages,
        model,
        fetcher,
        options: AssistantOptions {
            model,
            forward_function_call_id: model.is_some_and(is_gemini_3_or_newer),
        },
        history: History {
            contents: Vec::new(),
            warnings: Vec::new(),
        },
        last_tool_calls: None,
        tool_responses: Vec::new(),
        index: 0,
    };
    walker.run().await?;
    Ok(walker.history)
}

struct Walker<'walk, Source> {
    messages: &'walk [Message],
    model: Option<&'walk str>,
    fetcher: &'walk Source,
    options: AssistantOptions<'walk>,
    history: History,
    last_tool_calls: Option<LastToolCallMessage>,
    tool_responses: Vec<Object>,
    index: usize,
}

impl<'walk, Source> Walker<'walk, Source>
where
    Source: RemoteMediaFetcher + Sync,
{
    async fn run(&mut self) -> Result<(), HistoryError> {
        while self.index < self.messages.len() {
            let initial = self.index;
            self.user_turn()?;
            self.assistant_turn()?;
            self.tool_result().await?;
            self.flush_tool_responses(false);
            if self.index == initial {
                let role = self
                    .messages
                    .get(self.index)
                    .map_or_else(String::new, |message| message.role.clone());
                return Err(HistoryError::Invalid(format!(
                    "message {} has role {role}, which a Gemini history cannot carry",
                    self.index
                )));
            }
        }
        self.finish();
        Ok(())
    }

    fn take_run(&mut self, roles: &[&str]) -> &'walk [Message] {
        let rest = self.messages.get(self.index..).unwrap_or_default();
        let length = rest
            .iter()
            .take_while(|current| roles.contains(&current.role.as_str()))
            .count();
        self.index = self.index.saturating_add(length);
        rest.get(..length).unwrap_or_default()
    }

    fn push_turn(&mut self, role: Role, parts: Vec<Object>) {
        if !parts.is_empty() {
            self.history.contents.push(Content { role, parts });
        }
    }

    fn user_turn(&mut self) -> Result<(), HistoryError> {
        let mut parts = Vec::new();
        for current in self.take_run(&[USER, SYSTEM]) {
            append_user_parts(current, self.model, &mut parts)?;
        }
        if !parts.is_empty() && !parts.iter().any(|part| part.contains_key("text")) {
            self.history.warnings.push(HistoryWarning::BlankText);
            tracing::warn!("a user turn has no text part; a space part is added");
            parts.push(object!("text" => Json::Text(SPACE.to_owned())));
        }
        self.push_turn(Role::User, parts);
        Ok(())
    }

    fn assistant_turn(&mut self) -> Result<(), HistoryError> {
        let mut parts = Vec::new();
        for current in self.take_run(&[ASSISTANT]) {
            append_assistant_parts(current, self.options, &mut parts, &mut self.last_tool_calls)?;
        }
        self.push_turn(Role::Model, parts);
        Ok(())
    }

    async fn tool_result(&mut self) -> Result<(), HistoryError> {
        let Some(current) = self.messages.get(self.index) else {
            return Ok(());
        };
        if !matches!(current.role.as_str(), TOOL | FUNCTION) {
            return Ok(());
        }
        let converted = convert_tool_result_to_gemini(
            current,
            self.last_tool_calls.as_ref(),
            self.options.forward_function_call_id,
            self.fetcher,
        )
        .await?;
        self.index = self.index.saturating_add(1);
        match converted {
            GeminiToolResult::Single(part) => self.tool_responses.push(object_of(part)?),
            GeminiToolResult::WithMedia(parts) => {
                for part in parts {
                    self.tool_responses.push(object_of(part)?);
                }
            }
        }
        Ok(())
    }

    fn flush_tool_responses(&mut self, trailing: bool) {
        let tool_role_next = self
            .messages
            .get(self.index)
            .is_some_and(|current| matches!(current.role.as_str(), TOOL | FUNCTION));
        if (!trailing && tool_role_next) || self.tool_responses.is_empty() {
            return;
        }
        self.history.contents.push(Content {
            role: Role::User,
            parts: take(&mut self.tool_responses),
        });
    }

    fn finish(&mut self) {
        self.flush_tool_responses(true);
        if self.history.contents.is_empty() {
            self.history.warnings.push(HistoryWarning::EmptyHistory);
            tracing::warn!("the history has no content; a single space turn is sent instead");
            self.history.contents.push(Content {
                role: Role::User,
                parts: vec![object!("text" => Json::Text(SPACE.to_owned()))],
            });
        }
    }
}
