use betterlite_prompts::arguments::json::Json;

use crate::api::summary_history::boundary::strip_compaction_blocks;
use crate::api::summary_history::system::{
    SystemPrompt, system_to_openai_message, text_part, text_turn,
};
use crate::api::summary_history::translate::HistoryTranslator;

#[must_use]
pub fn append_text_to_content(content: Option<&Json>, extra_text: &str) -> Json {
    match content {
        None | Some(Json::Null) => Json::Text(extra_text.to_owned()),
        Some(Json::Text(text)) if text.is_empty() => Json::Text(extra_text.to_owned()),
        Some(Json::Text(text)) => Json::Text(format!("{text}\n\n{extra_text}")),
        Some(Json::List(parts)) => {
            let mut appended = parts.clone();
            appended.push(text_part(extra_text));
            Json::List(appended)
        }
        Some(scalar @ (Json::Bool(_) | Json::Number(_) | Json::Object(_))) => {
            Json::List(vec![scalar.clone(), text_part(extra_text)])
        }
    }
}

// strict OpenAI-compatible endpoints reject two user turns in a row, so the prompt joins a trailing user turn
pub async fn build_summary_messages<Translator>(
    messages: &[Json],
    prompt: &str,
    system: Option<&SystemPrompt>,
    translator: &Translator,
) -> Vec<Json>
where
    Translator: HistoryTranslator + Sync,
{
    let stripped = strip_compaction_blocks(messages);
    let translated = translator.messages(&stripped).await;
    let history = match translated {
        Ok(openai_shape) => openai_shape,
        Err(error) => {
            tracing::warn!(
                "compact_20260112: anthropic\u{2192}openai translation failed when building summary call; falling back to raw shape: {error}"
            );
            stripped
        }
    };
    let mut summary_messages: Vec<Json> = system_to_openai_message(system).into_iter().collect();
    summary_messages.extend(history);
    match summary_messages.last_mut() {
        Some(Json::Object(last)) if last.get("role").and_then(Json::as_str) == Some("user") => {
            let content = append_text_to_content(last.get("content"), prompt);
            last.insert("content".to_owned(), content);
        }
        Some(_) | None => summary_messages.push(text_turn("user", prompt)),
    }
    summary_messages
}
