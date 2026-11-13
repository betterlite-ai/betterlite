use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::langfuse_rest::client::LangfuseApiClient;
use crate::langfuse_rest::records::{ChatPrompt, Prompt, PromptMessage, PromptMeta, TextPrompt};

const LANGFUSE_INTEGRATION: &str = "langfuse";

// litellm.types.utils.StandardLoggingPromptManagementMetadata; the prompt variables never reach the span link
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct PromptManagementMetadata {
    pub prompt_id: String,
    pub prompt_variables: Option<Json>,
    pub prompt_integration: String,
}

// litellm.integrations.langfuse.langfuse.py._add_prompt_to_generation_params writes one of these under generation_params["prompt"]; only a client links the span
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationPrompt {
    Client(Box<Prompt>),
    Raw(Json),
}

// litellm.integrations.langfuse.langfuse.py._add_prompt_to_generation_params; the caller's prompt wins, a fetch failure stays non-blocking, and another value is kept as given
#[must_use]
pub async fn attach_generation_prompt(
    clean_metadata: &mut IndexMap<String, Json>,
    prompt_management_metadata: Option<&PromptManagementMetadata>,
    langfuse_client: &LangfuseApiClient,
) -> Option<GenerationPrompt> {
    let user_prompt = clean_metadata.shift_remove("prompt").unwrap_or(Json::Null);
    if user_prompt == Json::Null && prompt_management_metadata.is_none() {
        return None;
    }
    if let Json::Object(fields) = &user_prompt {
        return prompt_from_fields(fields);
    }
    match prompt_management_metadata {
        Some(metadata) if metadata.prompt_integration == LANGFUSE_INTEGRATION => {
            match langfuse_client
                .get_prompt(&metadata.prompt_id, None, None)
                .await
            {
                Ok(prompt) => Some(GenerationPrompt::Client(Box::new(prompt))),
                Err(error) => {
                    tracing::debug!(
                        "the Langfuse prompt could not be fetched for logging: {error}"
                    );
                    None
                }
            }
        }
        Some(_) | None => Some(GenerationPrompt::Raw(user_prompt)),
    }
}

fn prompt_from_fields(fields: &IndexMap<String, Json>) -> Option<GenerationPrompt> {
    let prompt_type = match fields.get("type") {
        Some(Json::Text(kind)) => Some(kind.as_str()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    };
    if matches!(prompt_type, Some("chat" | "text")) {
        return decode(fields, "the caller's prompt")
            .map(|prompt| GenerationPrompt::Client(Box::new(prompt)));
    }
    if fields.contains_key("version") && fields.contains_key("prompt") {
        return versioned_prompt(fields);
    }
    tracing::error!("the prompt metadata is not a Langfuse prompt format, so no prompt is logged");
    None
}

fn decode<T, V>(value: &V, what: &str) -> Option<T>
where
    T: DeserializeOwned,
    V: Serialize + ?Sized,
{
    let encoded = match serde_json::to_value(value) {
        Ok(encoded) => encoded,
        Err(error) => {
            tracing::error!("{what} could not be read: {error}");
            return None;
        }
    };
    match serde_json::from_value(encoded) {
        Ok(decoded) => Some(decoded),
        Err(error) => {
            tracing::error!("{what} is not a Langfuse prompt: {error}");
            None
        }
    }
}

fn versioned_prompt(fields: &IndexMap<String, Json>) -> Option<GenerationPrompt> {
    let meta = versioned_meta(fields)?;
    match fields.get("prompt") {
        Some(Json::Text(prompt)) => Some(GenerationPrompt::Client(Box::new(Prompt::Text(
            TextPrompt {
                prompt: prompt.clone(),
                meta,
            },
        )))),
        Some(Json::List(messages)) => {
            let chat_messages: Vec<PromptMessage> = decode(messages, "the caller's chat prompt")?;
            Some(GenerationPrompt::Client(Box::new(Prompt::Chat(
                ChatPrompt {
                    prompt: chat_messages,
                    meta,
                },
            ))))
        }
        Some(_) | None => {
            tracing::error!(
                "the caller's prompt content is neither text nor chat, so no prompt is logged"
            );
            None
        }
    }
}

fn versioned_meta(fields: &IndexMap<String, Json>) -> Option<PromptMeta> {
    let Some(Json::Text(name)) = fields.get("name") else {
        tracing::error!("the caller's prompt has no name, so no prompt is logged");
        return None;
    };
    let Some(Json::Number(version)) = fields.get("version") else {
        tracing::error!("the caller's prompt has no version, so no prompt is logged");
        return None;
    };
    let Some(version_number) = version.as_i64() else {
        tracing::error!(
            "the caller's prompt version is not a whole number, so no prompt is logged"
        );
        return None;
    };
    let Some(labels) = string_list(fields.get("labels")) else {
        tracing::error!(
            "the caller's prompt labels are not a list of strings, so no prompt is logged"
        );
        return None;
    };
    let Some(tags) = string_list(fields.get("tags")) else {
        tracing::error!(
            "the caller's prompt tags are not a list of strings, so no prompt is logged"
        );
        return None;
    };
    Some(PromptMeta {
        name: name.clone(),
        version: version_number,
        config: fields.get("config").cloned().unwrap_or(Json::Null),
        labels,
        tags,
        commit_message: None,
        resolution_graph: None,
    })
}

// python's `labels or []`: absent and null both count as no labels, anything else must be a list of strings
fn string_list(value: Option<&Json>) -> Option<Vec<String>> {
    match value {
        None | Some(Json::Null) => Some(Vec::new()),
        Some(Json::List(items)) => items
            .iter()
            .map(|item| match item {
                Json::Text(text) => Some(text.clone()),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => None,
    }
}
