use std::future::Future;

use betterlite_core::api::summary_history::error::SummaryHistoryError;
use betterlite_core::api::summary_history::translate::{AdapterTranslator, HistoryTranslator};
use betterlite_prompts::arguments::json::Json;
use thiserror::Error;

use super::input::{Outcome, Translation};

#[derive(Debug, Error)]
pub(super) enum TranslationFailure {
    // the case makes LiteLLM's adapter raise ValueError("broken"), so both sides log the same text
    #[error("broken")]
    Broken,
    #[error(transparent)]
    Adapter(#[from] SummaryHistoryError),
}

pub(super) struct CaseTranslator {
    pub(super) translation: Translation,
}

impl HistoryTranslator for CaseTranslator {
    type Error = TranslationFailure;

    fn messages(
        &self,
        messages: &[Json],
    ) -> impl Future<Output = Result<Vec<Json>, TranslationFailure>> + Send {
        let outcome = self.translation.messages;
        async move {
            match outcome {
                Outcome::Broken => Err(TranslationFailure::Broken),
                Outcome::Translated => Ok(AdapterTranslator.messages(messages).await?),
            }
        }
    }

    fn tools(&self, tools: &[Json]) -> Result<Vec<Json>, TranslationFailure> {
        match self.translation.tools {
            Outcome::Broken => Err(TranslationFailure::Broken),
            Outcome::Translated => Ok(AdapterTranslator.tools(tools)?),
        }
    }
}
