mod tests;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, MutexGuard};

use rust_decimal::Decimal;
use thiserror::Error;

use crate::deepagents::transformation::message::{Message, MessageUsage};
use crate::types::Usage;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LlmResult {
    pub generations: Vec<Vec<Generation>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Generation {
    pub message: Option<Message>,
}

// the price source a session injects; an unknown model answers None and its calls cost zero
pub trait TokenPricer: Send + Sync {
    fn cost_per_token(
        &self,
        model: &str,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> Option<Decimal>;
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AccountingError {
    #[error("the model call count overflows")]
    CallsOverflow,
    #[error("the token count overflows")]
    TokensOverflow,
    #[error("the cost overflows")]
    CostOverflow,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct UsageLedger {
    usage: Usage,
    cost: Decimal,
    unpriced: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UsageTotals {
    pub usage: Usage,
    pub cost: Decimal,
}

#[must_use]
pub fn message_cost(
    message: &Message,
    cost_model: Option<&str>,
    tokens: MessageUsage,
    pricer: &dyn TokenPricer,
) -> Option<Decimal> {
    if let Some(reported) = message.reported_cost {
        return Some(reported);
    }
    let model = cost_model?;
    pricer.cost_per_token(model, tokens.input_tokens, tokens.output_tokens)
}

pub(crate) fn record_llm_usage(
    ledger: &mut UsageLedger,
    cost_model: Option<&str>,
    result: &LlmResult,
    pricer: &dyn TokenPricer,
) -> Result<(), AccountingError> {
    ledger.usage.calls = ledger
        .usage
        .calls
        .checked_add(1)
        .ok_or(AccountingError::CallsOverflow)?;
    for generation in result.generations.iter().flatten() {
        let Some(message) = generation.message.as_ref() else {
            continue;
        };
        let tokens = message.usage;
        ledger.usage.input_tokens = ledger
            .usage
            .input_tokens
            .checked_add(tokens.input_tokens)
            .ok_or(AccountingError::TokensOverflow)?;
        ledger.usage.output_tokens = ledger
            .usage
            .output_tokens
            .checked_add(tokens.output_tokens)
            .ok_or(AccountingError::TokensOverflow)?;
        let priced_model = cost_model.filter(|model| !ledger.unpriced.contains(*model));
        match message_cost(message, priced_model, tokens, pricer) {
            Some(cost) => {
                ledger.cost = ledger
                    .cost
                    .checked_add(cost)
                    .ok_or(AccountingError::CostOverflow)?;
            }
            None => {
                if let Some(model) = priced_model {
                    ledger.unpriced.insert(model.to_owned());
                    tracing::warn!(
                        model = %model,
                        "harness deepagents: no cost for this model; counting its calls as zero"
                    );
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct UsageCallback {
    cost_model: Option<String>,
    pricer: Arc<dyn TokenPricer>,
    ledger: Arc<Mutex<UsageLedger>>,
}

impl UsageCallback {
    #[must_use]
    pub fn new(cost_model: Option<String>, pricer: Arc<dyn TokenPricer>) -> Self {
        Self {
            cost_model,
            pricer,
            ledger: Arc::new(Mutex::new(UsageLedger::default())),
        }
    }

    fn lock(&self) -> MutexGuard<'_, UsageLedger> {
        match self.ledger.lock() {
            Ok(ledger) => ledger,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    pub fn record(&self, result: &LlmResult) {
        let mut ledger = self.lock();
        if let Err(error) = record_llm_usage(
            &mut ledger,
            self.cost_model.as_deref(),
            result,
            self.pricer.as_ref(),
        ) {
            tracing::error!(error = %error, "harness deepagents: usage accounting failed");
        }
    }

    #[must_use]
    pub fn totals(&self) -> UsageTotals {
        let ledger = self.lock();
        UsageTotals {
            usage: ledger.usage,
            cost: ledger.cost,
        }
    }
}
