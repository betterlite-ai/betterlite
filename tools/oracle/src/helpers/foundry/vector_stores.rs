mod fixtures;
mod reports;
mod scenarios;
mod tests;

use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use scenarios::{
    AuthCredentialsInput, AzureSearchEndpointInput, DocumentScoreCitationInput,
    SearchQueryFieldsInput, StoreTypeEndpointsInput, UnsupportedStoreCreateInput,
    VectorQueryWithEmbeddingInput,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    StoreTypeEndpoints(StoreTypeEndpointsInput),
    AuthCredentials(AuthCredentialsInput),
    SearchQueryFields(SearchQueryFieldsInput),
    VectorQueryWithEmbedding(VectorQueryWithEmbeddingInput),
    DocumentScoreCitation(DocumentScoreCitationInput),
    UnsupportedStoreCreate(UnsupportedStoreCreateInput),
    AzureSearchEndpoint(AzureSearchEndpointInput),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        reports::run(input)
    }
}
