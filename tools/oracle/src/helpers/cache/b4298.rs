#[path = "b4298/cases.rs"]
mod cases;
mod sinks;
#[path = "b4298/support.rs"]
mod support;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    ConvertToCachedEmbedding(cases::ConvertInput),
    GetPerItemPromptTokens(cases::TokenInput),
    GetPerItemPromptTokensDetails(cases::TokenInput),
    AddEmbeddingResponseToCache(cases::EntryInput),
    AsyncAddCachePipeline(cases::PipelineInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ConvertToCachedEmbedding(case) => Ok(cases::convert(case)),
            Input::GetPerItemPromptTokens(case) => Ok(cases::prompt_tokens(case)),
            Input::GetPerItemPromptTokensDetails(case) => Ok(cases::prompt_tokens_details(case)),
            Input::AddEmbeddingResponseToCache(case) => cases::add_entry(case),
            Input::AsyncAddCachePipeline(case) => cases::pipeline(case),
        }
    }
}
