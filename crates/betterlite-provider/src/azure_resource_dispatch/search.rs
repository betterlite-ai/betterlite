use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_azure::search::AzureSearchConfig;
use betterlite_azure::search::error::AzureSearchError;
use betterlite_azure::search::reply::AzureSearchReply;
use betterlite_azure::search::request::AzureSearchRequest;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::SearchQuery;
use betterlite_openai::search::records::SearchResponse;
use secrecy::SecretString;

pub struct Dispatch<'call> {
    pub config: &'call AzureSearchConfig<'call>,
    pub call: Call<'call>,
}

pub enum Call<'call> {
    ValidateEnvironment {
        headers: Headers,
        api_key: Option<&'call SecretString>,
        api_base: Option<&'call str>,
    },
    CompleteUrl(Option<&'call str>),
    TransformRequest {
        query: &'call SearchQuery,
        optional_params: &'call Object,
    },
    TransformResponse {
        reply: &'call AzureSearchReply<'call>,
        optional_params: Option<&'call Object>,
    },
}

pub enum Outcome {
    Headers(Headers),
    Url(String),
    Request(Box<AzureSearchRequest>),
    Response(Box<SearchResponse>),
}

pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, AzureSearchError>> + Send + 'call>>;

impl Dispatch<'_> {
    // B546's grounded-search calls, all served by the one configured client
    pub fn run(self) -> Result<Outcome, AzureSearchError> {
        let Self { config, call } = self;
        Ok(match call {
            Call::ValidateEnvironment {
                headers,
                api_key,
                api_base,
            } => Outcome::Headers(config.validate_environment(headers, api_key, api_base)?),
            Call::CompleteUrl(api_base) => Outcome::Url(config.complete_url(api_base)?),
            Call::TransformRequest {
                query,
                optional_params,
            } => Outcome::Request(Box::new(
                config.transform_search_request(query, optional_params)?,
            )),
            Call::TransformResponse {
                reply,
                optional_params,
            } => Outcome::Response(Box::new(
                config.transform_search_response(reply, optional_params)?,
            )),
        })
    }
}

pub fn dispatch(dispatch: Dispatch<'_>) -> RunFuture<'_> {
    Box::pin(ready(dispatch.run()))
}
