#[path = "b4298/cases.rs"]
mod cases;
#[path = "b4298/support.rs"]
mod support;

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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::ConvertToCachedEmbedding(case) => Ok(cases::convert(case)),
            Input::GetPerItemPromptTokens(case) => Ok(cases::prompt_tokens(case)),
            Input::GetPerItemPromptTokensDetails(case) => Ok(cases::prompt_tokens_details(case)),
            Input::AddEmbeddingResponseToCache(case) => cases::add_entry(case),
            Input::AsyncAddCachePipeline(case) => cases::pipeline(case),
        }
    }
}
