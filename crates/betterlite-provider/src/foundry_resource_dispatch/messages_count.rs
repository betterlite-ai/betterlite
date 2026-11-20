use std::future::{Future, ready};
use std::pin::Pin;

use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_foundry::anthropic_count::endpoint::get_count_tokens_endpoint;
use betterlite_foundry::anthropic_count::error::ApiError;
use betterlite_foundry::anthropic_count::handler::{CountTokensRequest, FoundryCountHandler};
use betterlite_foundry::anthropic_count::reply::FoundryCountReply;
use betterlite_foundry::anthropic_messages::transform::transform_request;
use betterlite_foundry::anthropic_messages::url::get_complete_url;
use betterlite_foundry::anthropic_messages::url_error::UrlError;
use betterlite_foundry::anthropic_messages::{CUSTOM_LLM_PROVIDER, SHOULD_STRIP_BILLING_METADATA};
use betterlite_prompts::arguments::json::Object;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Url(#[from] UrlError),
    #[error(transparent)]
    Count(#[from] ApiError),
}

pub type RunFuture<'call> = Pin<Box<dyn Future<Output = Result<Outcome, Error>> + Send + 'call>>;

// B564's native Messages endpoint and B565's token count, which runs through the handler's transport
pub struct Dispatch<'call, T> {
    pub handler: &'call FoundryCountHandler<T>,
    pub call: Call<'call>,
}

pub enum Call<'call> {
    Policy,
    MessagesUrl {
        api_base: Option<&'call str>,
        environment_api_base: Option<&'call str>,
    },
    Transform {
        request: &'call mut Object,
    },
    CountUrl {
        api_base: &'call str,
    },
    Count(CountTokensRequest<'call>),
}

pub enum Outcome {
    Policy {
        provider: &'static str,
        strips_billing_metadata: bool,
    },
    MessagesUrl(String),
    Transformed(Object),
    CountUrl(String),
    Count(FoundryCountReply),
}

impl<'call, T: CountTokensTransport> Dispatch<'call, T> {
    // The count borrows credential sources that are not Sync, so only the handler's Send future is boxed
    pub fn run(self) -> RunFuture<'call> {
        let Self { handler, call } = self;
        match call {
            Call::Policy => Box::pin(ready(Ok(Outcome::Policy {
                provider: CUSTOM_LLM_PROVIDER,
                strips_billing_metadata: SHOULD_STRIP_BILLING_METADATA,
            }))),
            Call::MessagesUrl {
                api_base,
                environment_api_base,
            } => Box::pin(ready(
                get_complete_url(api_base, environment_api_base)
                    .map(Outcome::MessagesUrl)
                    .map_err(Error::Url),
            )),
            Call::Transform { request } => {
                transform_request(&mut *request);
                Box::pin(ready(Ok(Outcome::Transformed(request.clone()))))
            }
            Call::CountUrl { api_base } => Box::pin(ready(Ok(Outcome::CountUrl(
                get_count_tokens_endpoint(api_base),
            )))),
            Call::Count(request) => {
                let started = handler.handle_count_tokens_request(&request);
                Box::pin(async move { started.await.map(Outcome::Count).map_err(Error::Count) })
            }
        }
    }
}
