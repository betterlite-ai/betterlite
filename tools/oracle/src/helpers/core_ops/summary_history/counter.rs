use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::summary_history::count::{
    CountedInput, HistoryTokenCounter, TokenizerHistoryCounter,
};
use betterlite_core::api::summary_history::error::SummaryHistoryError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Json;
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::runtime::runtime_counter;
use serde::Serialize;

use super::input::CounterInput;

#[derive(Debug, Serialize)]
pub(super) struct CallRecord {
    model: String,
    messages: Option<Vec<Json>>,
    tools: Option<Vec<Json>>,
    text: Option<String>,
    count: usize,
}

pub(super) enum CountSource {
    Scripted(Box<Mutex<VecDeque<usize>>>),
    Real(TokenizerHistoryCounter),
}

pub(super) struct RecordingCounter {
    pub(super) source: CountSource,
    pub(super) calls: Mutex<Vec<CallRecord>>,
}

impl CountSource {
    pub(super) fn from_case(counter: &CounterInput) -> Result<Self, Box<HelperError>> {
        match counter {
            CounterInput::Scripted { values } => Ok(Self::Scripted(Box::new(Mutex::new(
                values.iter().copied().collect(),
            )))),
            CounterInput::Real => {
                let config = ConfigLoader {
                    lookup: BTreeMap::<String, String>::new(),
                }
                .load("general_settings:\n  master_key: oracle-master\n")
                .or_value_error()?;
                let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
                    .or_value_error()?;
                let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).or_value_error()?);
                let catalog = BundledCatalog::default().get();
                let built = runtime_counter(&config, &client, &catalog).or_value_error()?;
                Ok(Self::Real(TokenizerHistoryCounter {
                    counter: Arc::new(built),
                }))
            }
        }
    }
}

impl HistoryTokenCounter for RecordingCounter {
    async fn count(
        &self,
        model: &str,
        input: CountedInput<'_>,
    ) -> Result<usize, SummaryHistoryError> {
        // LiteLLM's real token_counter stays unwrapped on the Python side, so only scripted calls are recorded
        let counted = match &self.source {
            CountSource::Scripted(values) => {
                let next = values
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pop_front();
                next.ok_or(SummaryHistoryError::TokenCount(CountError::Overflow))?
            }
            CountSource::Real(counter) => return counter.count(model, input).await,
        };
        let (messages, tools, text) = match input {
            CountedInput::Conversation {
                messages: history,
                tools: declared,
            } => (Some(history.to_vec()), declared.map(<[Json]>::to_vec), None),
            CountedInput::Text(counted_text) => (None, None, Some(counted_text.to_owned())),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CallRecord {
                model: model.to_owned(),
                messages,
                tools,
                text,
                count: counted,
            });
        Ok(counted)
    }
}
