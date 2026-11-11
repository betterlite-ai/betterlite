use std::error::Error as StdError;
use std::future::Future;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::datadog_llm_obs::records::DdMessage;

// LiteLLM reaches the tokenizer only through token_counter(model, text), so the caller injects it
pub trait ToolTokenCounter: Sync {
    type Failure: StdError + Send + Sync + 'static;

    fn count_text(
        &self,
        model: &str,
        text: &str,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum ToolTokensError<Failure>
where
    Failure: StdError + 'static,
{
    #[error("the token counter failed on a tool result")]
    Counter {
        #[source]
        source: Failure,
    },
    #[error("the tool output token count does not fit a span metric")]
    Overflow,
}

pub async fn tool_output_tokens<Counter>(
    messages: &[DdMessage],
    model: &str,
    counter: &Counter,
) -> Result<Option<f64>, ToolTokensError<Counter::Failure>>
where
    Counter: ToolTokenCounter,
{
    let mut total = 0_u64;
    let mut counted_any = false;
    for entry in messages {
        let DdMessage::Message(record) = entry else {
            continue;
        };
        for result in &record.tool_results {
            counted_any = true;
            let tokens = counter
                .count_text(model, &result.result)
                .await
                .map_err(|source| ToolTokensError::Counter { source })?;
            let count = u64::try_from(tokens).map_err(|_conversion| ToolTokensError::Overflow)?;
            total = total.checked_add(count).ok_or(ToolTokensError::Overflow)?;
        }
    }
    if !counted_any {
        return Ok(None);
    }
    let tokens = Decimal::from(total)
        .to_f64()
        .ok_or(ToolTokensError::Overflow)?;
    Ok(Some(tokens))
}

mod tests;
