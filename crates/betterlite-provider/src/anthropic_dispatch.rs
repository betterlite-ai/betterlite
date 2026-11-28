use std::error::Error as StdError;
pub mod base;
pub mod config;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use futures_util::stream::BoxStream;

use crate::operation::{
    ChatOperation, OperationError, OperationRequest, OperationResult, RequestKind,
};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Adapter;
use base::Surface;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anthropic {
    pub surface: Surface,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicRequest;

// The client's reply or the framed stream; the B010 stub it replaces had no payload.
pub enum AnthropicResult {
    Reply(Box<gateway::reply::AnthropicReply>),
    Stream(BoxStream<'static, Result<Vec<u8>, HttpError>>),
}

// The chat call B027 runs, plus the stream flag its surface needs at execute time.
#[derive(Debug, Clone)]
pub struct AnthropicPrepared {
    pub chat: Box<ChatOperation>,
    pub stream: bool,
}

impl Adapter for Anthropic {
    const NAME: &'static str = "anthropic";

    fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        _catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        if self.surface == Surface::Legacy || deployment.anthropic.is_none() {
            return Err(OperationError::UnavailableAdapter {
                provider: String::from(Self::NAME),
            });
        }
        match request.kind() {
            RequestKind::Chat(chat) => Ok(PreparedRequest::Anthropic(AnthropicPrepared {
                chat: Box::new(chat.clone()),
                stream: false,
            })),
            RequestKind::Stream(chat) => Ok(PreparedRequest::Anthropic(AnthropicPrepared {
                chat: Box::new(chat.clone()),
                stream: true,
            })),
            RequestKind::Embedding(_) | RequestKind::Other => {
                Err(OperationError::UnsupportedOperation {
                    provider: String::from(Self::NAME),
                    operation: String::from("provider endpoint"),
                })
            }
        }
    }

    fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        deployment: &Deployment,
    ) -> impl Future<Output = Result<Upstream, OperationError>> + Send {
        let unavailable = self.surface == Surface::Legacy || deployment.anthropic.is_none();
        async move {
            if unavailable {
                return Err(OperationError::UnavailableAdapter {
                    provider: String::from(Self::NAME),
                });
            }
            gateway::call::execute(client, prepared, deployment).await
        }
    }

    fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        if self.surface == Surface::Legacy {
            return Err(OperationError::UnavailableAdapter {
                provider: String::from(Self::NAME),
            });
        }
        gateway::reply::decode(prepared, upstream)
    }
}

pub mod completion;
#[cfg(any(test, feature = "testing"))]
pub mod fixtures;
pub mod gateway;
pub mod native;
pub mod operations;
mod tests;

use betterlite_anthropic::batches::config::BatchesConfig as _;
use betterlite_anthropic::batches::error::BatchesError;
use betterlite_anthropic::batches::handler::BatchTransport;
use betterlite_anthropic::count_tokens::error::ApiError;
use betterlite_anthropic::count_tokens::handler::CountTokensTransport;
use betterlite_anthropic::files::error::FilesError;
use betterlite_anthropic::response::Logging;
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_openai::files::transport::FileTransport;
use operations::{DispatchDeps, Operation, Outcome};

#[derive(Debug, thiserror::Error)]
pub enum DispatchError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Selection(config::ConfigSelectionError),
    #[error(transparent)]
    Count(#[from] ApiError),
    #[error(transparent)]
    Batch(#[from] BatchesError),
    #[error(transparent)]
    File(#[from] FilesError),
    #[error("the completion handler failed")]
    Completion(#[source] Failure),
}

// Each arm installs its native leaf; completion bindings carry the selected config to B584/B010.
#[rustfmt::skip]
pub fn dispatch<'call, Client, Encoding, Transport, Handler>(
    operation: Operation<'call, Client, Encoding>,
    deps: &'call mut DispatchDeps<'_, Transport, Handler>,
    log: &'call mut LogContext<'_, '_>,
) -> completion::HandlerFuture<'call, Outcome<Handler::Reply>, DispatchError<Handler::Failure>>
where
    Client: Send + 'call,
    Encoding: Send + 'call,
    Transport: CountTokensTransport + BatchTransport + FileTransport,
    Handler: completion::CompletionHandler<Client, Encoding> + Send,
{
    Box::pin(async move {
        match operation {
            Operation::Completion(surface, call) => Ok(Outcome::Completion(completion::complete(surface, *call, deps.completion, deps.handler, log).await?)),
            Operation::Count(call) => Ok(Outcome::Count(deps.count.handle_count_tokens_request(&call).await?)),
            Operation::BatchCreate => {
                deps.batches.config.transform_create_batch_request()?;
                Ok(Outcome::Batch(Box::new(deps.batches.config.transform_create_batch_response()?)))
            }
            Operation::BatchRetrieve(call) => Ok(Outcome::Batch(Box::new(deps.batches.retrieve_batch(&call, Logging { state: log.state, env: log.env }).await?))),
            Operation::BatchResults(call) | Operation::Content(call) => Ok(Outcome::Content(deps.files.content(deps.file_client, &call).await?)),
            Operation::Upload(call) => Ok(Outcome::File(Box::new(deps.files.create_file(deps.file_client, &call).await?))),
            Operation::Retrieve(call) => Ok(Outcome::File(Box::new(deps.files.retrieve_file(deps.file_client, &call).await?))),
            Operation::List(call) => Ok(Outcome::Files(deps.files.list_files(deps.file_client, &call).await?)),
            Operation::Delete(call) => Ok(Outcome::Deleted(deps.files.delete_file(deps.file_client, &call).await?)),
            Operation::ConfigUrl(url) => Ok(Outcome::Config(config::get_anthropic_config(url))),
        }
    })
}
