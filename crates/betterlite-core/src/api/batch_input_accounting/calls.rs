use std::collections::BTreeMap;
use std::error::Error;

use super::ports::{BatchAccountingCalls, BatchFetchParams};
use crate::api::batch_limit_descriptors::answer::BatchLimitAnswer;
use crate::api::batch_limit_descriptors::input::{
    BatchLimitCall, BatchLimitDescriptorsInput, BatchLimitPrincipal,
};
use crate::api::batch_limit_descriptors::limiter::BatchLimiterSource;
use crate::api::batch_limit_descriptors::{BatchLimitDescriptors, BatchLimitError};
use crate::api::model_credentials::CredentialSource;
use crate::api::request_destination::RequestData;

// B1044 answers one call per run, so every failure is either its own error or a mismatched answer
#[derive(Debug, thiserror::Error)]
pub enum BatchAccountingCallError<LimiterError, CredentialError>
where
    LimiterError: Error + 'static,
    CredentialError: Error + 'static,
{
    #[error(transparent)]
    Descriptors(#[from] BatchLimitError<LimiterError, CredentialError>),
    #[error("the descriptor service answered a different batch call")]
    UnexpectedAnswer,
}

// The row estimate and fetch-params answers read no principal, so the call runs under an empty one
fn answer_one<Limiter, Credentials>(
    service: &BatchLimitDescriptors<Limiter, Credentials>,
    call: BatchLimitCall,
) -> Result<BatchLimitAnswer, BatchAccountingCallError<Limiter::Error, Credentials::Error>>
where
    Limiter: BatchLimiterSource,
    Credentials: CredentialSource,
{
    let input = BatchLimitDescriptorsInput {
        principal: BatchLimitPrincipal(BTreeMap::new()),
        calls: vec![call],
    };
    let mut answers = service.run(&input)?.answers;
    let Some(answer) = answers.pop() else {
        return Err(BatchAccountingCallError::UnexpectedAnswer);
    };
    Ok(answer)
}

impl<Limiter, Credentials> BatchAccountingCalls for BatchLimitDescriptors<Limiter, Credentials>
where
    Limiter: BatchLimiterSource + Send + Sync,
    Credentials: CredentialSource + Send + Sync,
{
    type Error = BatchAccountingCallError<Limiter::Error, Credentials::Error>;

    fn output_floor(&self, min_configured_otpm_limit: Option<i64>) -> i64 {
        self.limiter.output_floor(min_configured_otpm_limit)
    }

    fn estimate_output_tokens(
        &self,
        entry: &RequestData,
        min_configured_otpm_limit: Option<i64>,
    ) -> Result<i64, Self::Error> {
        let answer = answer_one(
            self,
            BatchLimitCall::EstimateOutputTokens {
                entry: entry.clone(),
                min_configured_otpm_limit,
            },
        )?;
        let BatchLimitAnswer::EstimateOutputTokens { tokens } = answer else {
            return Err(BatchAccountingCallError::UnexpectedAnswer);
        };
        Ok(tokens)
    }

    fn resolve_fetch_params(
        &self,
        file_id: &str,
        custom_llm_provider: &str,
        data: &RequestData,
    ) -> Result<BatchFetchParams, Self::Error> {
        let answer = answer_one(
            self,
            BatchLimitCall::ResolveFetchParams {
                file_id: file_id.to_owned(),
                custom_llm_provider: custom_llm_provider.to_owned(),
                data: data.clone(),
            },
        )?;
        let BatchLimitAnswer::ResolveFetchParams { file_id, params } = answer else {
            return Err(BatchAccountingCallError::UnexpectedAnswer);
        };
        Ok(BatchFetchParams { file_id, params })
    }
}
