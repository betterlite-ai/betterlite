use std::error::Error;

use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::fields::usage::Usage;
use crate::fields::usage::parse::usage_from_mapping;
use crate::fields::usage::project::usage_json;
// ast-grep-ignore: no-long-names
use crate::logging_failure::partial_usage::record_partial_usage_for_failure;
use crate::logging_state::LoggingState;
use crate::logging_streams::cache_usage::backfill_cache_usage;

pub type PromptCounter<'counter> =
    dyn Fn() -> Result<i64, PromptCountError> + Send + Sync + 'counter;

#[derive(Debug, Error)]
pub enum PromptCountError {
    #[error("the prompt token count failed")]
    Failed(#[source] Box<dyn Error + Send + Sync>),
}

#[derive(Debug, Error)]
pub enum StreamAssemblyError {
    #[error("the streamed response could not be assembled")]
    Builder(#[source] Box<dyn Error + Send + Sync>),
}

// B2227's stream_chunk_builder over one stream's chunks, given the caller's prompt-token counter.
pub trait StreamAssembly {
    fn assemble(
        &self,
        count_prompt_tokens: Option<&PromptCounter<'_>>,
    ) -> Result<Option<Json>, StreamAssemblyError>;

    fn total_usage(&self) -> Usage;
}

// LiteLLM's end-of-stream assembly: the complete response, the usage-only fallback, or nothing.
pub enum StreamBuild {
    Complete(Json),
    UsageOnly(Box<Usage>),
    Empty,
}

#[must_use]
pub fn successful_stream_handoff(
    assembly: &dyn StreamAssembly,
    count_prompt_tokens: Option<&PromptCounter<'_>>,
) -> StreamBuild {
    match assembly.assemble(count_prompt_tokens) {
        Ok(Some(response)) => StreamBuild::Complete(response),
        Ok(None) => StreamBuild::Empty,
        Err(error) => {
            tracing::warn!(
                %error,
                "stream_chunk_builder raised at end-of-stream; logging best-effort usage from chunks"
            );
            StreamBuild::UsageOnly(Box::new(assembly.total_usage()))
        }
    }
}

// LiteLLM's _record_partial_usage_for_failure: an interrupted stream still billed for its chunks.
#[must_use]
pub fn partial_stream_handoff<Cost>(
    assembly: &dyn StreamAssembly,
    count_prompt_tokens: Option<&PromptCounter<'_>>,
    state: &mut LoggingState,
    response_cost: Cost,
) -> Option<Decimal>
where
    Cost: FnOnce(&Json) -> Option<Decimal>,
{
    let response = match assembly.assemble(count_prompt_tokens) {
        Ok(Some(response)) => response,
        Ok(None) => return None,
        Err(error) => {
            tracing::debug!(%error, "could not recover partial usage for interrupted stream");
            return None;
        }
    };
    let mut usage = response_usage(&response)?;
    backfill_cache_usage(&mut usage);
    let priced = priced_response(response, &state.model, &usage);
    let cost = response_cost(&priced).unwrap_or(Decimal::ZERO);
    // ast-grep-ignore: no-long-names
    record_partial_usage_for_failure(state, &usage, cost);
    Some(cost)
}

fn response_usage(response: &Json) -> Option<Usage> {
    let Json::Object(fields) = response else {
        return None;
    };
    let Some(Json::Object(usage)) = fields.get("usage") else {
        return None;
    };
    usage_from_mapping(usage).ok()
}

// LiteLLM prices the partial response after the call's model and the backfilled usage are on it.
fn priced_response(mut response: Json, model: &str, usage: &Usage) -> Json {
    if let Json::Object(fields) = &mut response {
        if !model.is_empty() {
            fields.insert("model".to_owned(), Json::Text(model.to_owned()));
        }
        fields.insert("usage".to_owned(), usage_json(usage));
    }
    response
}
