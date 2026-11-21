use std::future::Future;
use std::sync::Arc;

use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::request::{Message, TextInput, TokenRequest};

use crate::api::messages_context_clear::result::CompactionBlock;
use crate::api::summary_history::boundary::strip_compaction_blocks;
use crate::api::summary_history::error::SummaryHistoryError;
use crate::api::summary_history::system::{SystemPrompt, system_to_text};
use crate::api::summary_history::translate::HistoryTranslator;

#[derive(Debug, Clone, Copy)]
pub enum CountedInput<'input> {
    Conversation {
        messages: &'input [Json],
        tools: Option<&'input [Json]>,
    },
    Text(&'input str),
}

pub trait HistoryTokenCounter {
    fn count(
        &self,
        model: &str,
        input: CountedInput<'_>,
    ) -> impl Future<Output = Result<usize, SummaryHistoryError>> + Send;
}

pub struct TokenizerHistoryCounter {
    pub counter: Arc<Counter>,
}

impl HistoryTokenCounter for TokenizerHistoryCounter {
    async fn count(
        &self,
        model: &str,
        input: CountedInput<'_>,
    ) -> Result<usize, SummaryHistoryError> {
        let (text, messages, tools) = match input {
            CountedInput::Text(raw_text) => (Some(TextInput::One(raw_text.to_owned())), None, None),
            CountedInput::Conversation {
                messages: history,
                tools: declared_tools,
            } => {
                let decoded: Vec<Message> =
                    serde_json::to_value(history).and_then(serde_json::from_value)?;
                let declared: Option<Vec<DecisionJson>> = declared_tools
                    .map(|raw| serde_json::to_value(raw).and_then(serde_json::from_value))
                    .transpose()?;
                (None, Some(decoded), declared)
            }
        };
        let request = TokenRequest {
            model,
            custom_tokenizer: None,
            text: text.as_ref(),
            messages: messages.as_deref(),
            count_response_tokens: Some(false),
            tools: tools.as_deref(),
            tool_choice: None,
        };
        Ok(self.counter.count(&request).await?)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TokenComponents<'input> {
    pub model: &'input str,
    pub messages: &'input [Json],
    pub compaction_block: Option<&'input CompactionBlock>,
    pub tools: Option<&'input [Json]>,
    pub system: Option<&'input SystemPrompt>,
}

// counts what the downstream call will carry: the stripped history, the tools, any summary and the system text
pub async fn count_effective_tokens<Translator, TokenCounter>(
    components: TokenComponents<'_>,
    translator: &Translator,
    counter: &TokenCounter,
) -> Result<usize, SummaryHistoryError>
where
    Translator: HistoryTranslator + Sync,
    TokenCounter: HistoryTokenCounter + Sync,
{
    let stripped = strip_compaction_blocks(components.messages);
    let translated = translator.messages(&stripped).await;
    let messages = match translated {
        Ok(openai_shape) => openai_shape,
        Err(error) => {
            tracing::debug!(
                "compact_20260112: anthropic→openai translation failed during token count, falling back to raw messages: {error}"
            );
            stripped
        }
    };
    let declared = components.tools.filter(|raw| !raw.is_empty());
    let tools = declared.map(|raw| {
        translator.tools(raw).unwrap_or_else(|error| {
            tracing::debug!(
                "compact_20260112: anthropic→openai tools translation failed during token count, falling back to raw tools: {error}"
            );
            raw.to_vec()
        })
    });
    let conversation = CountedInput::Conversation {
        messages: &messages,
        tools: tools.as_deref(),
    };
    let mut total = counter.count(components.model, conversation).await?;
    let summary = components
        .compaction_block
        .and_then(|block| block.content.as_deref())
        .filter(|content| !content.is_empty());
    if let Some(content) = summary {
        let counted = counter
            .count(components.model, CountedInput::Text(content))
            .await?;
        total = total.saturating_add(counted);
    }
    let system_text = system_to_text(components.system);
    if !system_text.is_empty() {
        let counted = counter
            .count(components.model, CountedInput::Text(&system_text))
            .await?;
        total = total.saturating_add(counted);
    }
    Ok(total)
}
