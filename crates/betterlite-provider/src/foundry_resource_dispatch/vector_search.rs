use betterlite_foundry::vector_stores::environment::{
    AuthCredentials, Params, auth_credentials, complete_url, endpoints_by_type,
};
use betterlite_foundry::vector_stores::error::AzureSearchError;
use betterlite_foundry::vector_stores::reply::{AzureSearchStoreReply, decode_azure_search_reply};
use betterlite_foundry::vector_stores::search::{
    AzureSearchStoreRequest, SearchCall, search_request,
};
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreIndexEndpoints, VectorStoreSearchResponse,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Search(#[from] AzureSearchError),
}

// B558's Azure AI Search steps: index endpoints, base URL, credential headers, the query and its reply
pub struct Dispatch<'call> {
    pub call: Call<'call>,
}

pub enum Call<'call> {
    Endpoints,
    Url {
        api_base: Option<&'call str>,
        params: &'call Params,
    },
    Auth {
        params: &'call Params,
    },
    Search(AzureSearchStoreRequest<'call>),
    Reply(AzureSearchStoreReply<'call>),
}

pub enum Outcome {
    Endpoints(VectorStoreIndexEndpoints),
    Url(String),
    Auth(AuthCredentials),
    Search(SearchCall),
    Reply(VectorStoreSearchResponse),
}

impl Dispatch<'_> {
    pub fn run(self) -> Result<Outcome, Error> {
        Ok(match self.call {
            Call::Endpoints => Outcome::Endpoints(endpoints_by_type()),
            Call::Url { api_base, params } => Outcome::Url(complete_url(api_base, params)?),
            Call::Auth { params } => Outcome::Auth(auth_credentials(params)?),
            Call::Search(request) => Outcome::Search(search_request(request)?),
            Call::Reply(reply) => Outcome::Reply(decode_azure_search_reply(&reply)?),
        })
    }
}
