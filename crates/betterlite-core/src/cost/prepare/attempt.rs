use betterlite_logging::fields::usage::Usage;
use betterlite_tokenizer::count::error::CountError;
use rust_decimal::Decimal;

use super::error::PrepareError;
use super::usage_counts::counted_usage;
use super::{
    AttemptBilling, AttemptOutcome, BillingTokenCounter, CompletionBillingInput,
    PreparedBillingAttempt,
};

pub(super) struct AttemptState {
    pub(super) custom_llm_provider: Option<String>,
    pub(super) region_name: Option<String>,
    pub(super) installed_usage: Option<Usage>,
}

pub(super) async fn prepare_attempt(
    input: &CompletionBillingInput<'_>,
    candidate: Option<&str>,
    state: &mut AttemptState,
    counter: &dyn BillingTokenCounter,
) -> PreparedBillingAttempt {
    let outcome = attempt_outcome(input, candidate, state, counter).await;
    PreparedBillingAttempt {
        model: candidate.map(str::to_owned),
        custom_llm_provider: state.custom_llm_provider.clone(),
        region_name: state.region_name.clone(),
        outcome,
    }
}

async fn attempt_outcome(
    input: &CompletionBillingInput<'_>,
    candidate: Option<&str>,
    state: &mut AttemptState,
    counter: &dyn BillingTokenCounter,
) -> AttemptOutcome {
    let Some(view) = input.response else {
        return token_outcome(input, candidate, counter).await;
    };
    let Some(usage) = view.usage() else {
        return token_outcome(input, candidate, counter).await;
    };
    let extracted = counted_usage(&usage, state.installed_usage.as_ref());
    if extracted.replaced.is_some() {
        state.installed_usage = extracted.replaced;
    }
    let counts = match extracted.counts {
        Ok(found) => found,
        Err(error) => return AttemptOutcome::Failed(error),
    };
    if let Err(error) = view.rebind(&mut state.custom_llm_provider, &mut state.region_name) {
        return AttemptOutcome::Failed(error);
    }
    AttemptOutcome::Ready(AttemptBilling {
        prompt_tokens: counts.prompt,
        completion_tokens: counts.completion,
        cache_creation_input_tokens: counts.cache_creation,
        cache_read_input_tokens: counts.cache_read,
        prompt_characters: None,
        completion_characters: None,
        transcription_duration: Decimal::ZERO,
        rerank_units: None,
    })
}

async fn token_outcome(
    input: &CompletionBillingInput<'_>,
    candidate: Option<&str>,
    counter: &dyn BillingTokenCounter,
) -> AttemptOutcome {
    let Some(model) = candidate else {
        return AttemptOutcome::Failed(PrepareError::ModelMissing);
    };
    let prompt = if !input.messages.is_empty() {
        counted(
            counter.count_messages(model, input.messages).await,
            "prompt_tokens",
        )
    } else if !input.prompt.is_empty() {
        counted(
            counter.count_text(model, input.prompt).await,
            "prompt_tokens",
        )
    } else {
        Ok(0)
    };
    let prompt_tokens = match prompt {
        Ok(found) => found,
        Err(error) => return AttemptOutcome::Failed(error),
    };
    let completion = counted(
        counter.count_text(model, input.completion).await,
        "completion_tokens",
    );
    let completion_tokens = match completion {
        Ok(found) => found,
        Err(error) => return AttemptOutcome::Failed(error),
    };
    AttemptOutcome::Ready(AttemptBilling {
        prompt_tokens,
        completion_tokens,
        cache_creation_input_tokens: 0,
        cache_read_input_tokens: 0,
        prompt_characters: None,
        completion_characters: None,
        transcription_duration: Decimal::ZERO,
        rerank_units: None,
    })
}

fn counted(result: Result<usize, CountError>, field: &'static str) -> Result<i64, PrepareError> {
    match i64::try_from(result?) {
        Ok(count) => Ok(count),
        Err(_conversion) => Err(PrepareError::Count { field }),
    }
}
