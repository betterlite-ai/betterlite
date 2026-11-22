use std::fmt::Display;
use std::future::Future;

use betterlite_prompts::arguments::json::Json;

use crate::api::messages_chat_content::{ChatTarget, translate_messages};
use crate::api::messages_chat_tools_schema::tools::tools_to_openai;
use crate::api::summary_history::error::SummaryHistoryError;
use crate::shadow_requests::surfaces::environment::NoCapabilities;

// a failed translation falls back to the raw shape, so the seam lets a caller see that path
pub trait HistoryTranslator {
    type Error: Display + Send;

    fn messages(
        &self,
        messages: &[Json],
    ) -> impl Future<Output = Result<Vec<Json>, Self::Error>> + Send;

    fn tools(&self, tools: &[Json]) -> Result<Vec<Json>, Self::Error>;
}

pub struct AdapterTranslator;

impl HistoryTranslator for AdapterTranslator {
    type Error = SummaryHistoryError;

    // midturn system turns stay in place, so no model capability is ever asked
    async fn messages(&self, messages: &[Json]) -> Result<Vec<Json>, SummaryHistoryError> {
        let target = ChatTarget {
            model: None,
            custom_llm_provider: None,
        };
        let mut capabilities = NoCapabilities;
        Ok(translate_messages(messages, &target, true, &mut capabilities).await?)
    }

    fn tools(&self, tools: &[Json]) -> Result<Vec<Json>, SummaryHistoryError> {
        Ok(tools_to_openai(tools, None)?.tools)
    }
}
