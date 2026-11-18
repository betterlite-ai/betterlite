pub mod base;
pub mod config;

use crate::provider::Adapter;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Anthropic;

// B010's stubs stay in the shape their variants were registered with until the composition fills them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicRequest;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicResult;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AnthropicPrepared;

impl Adapter for Anthropic {
    const NAME: &'static str = "anthropic";
}

pub mod completion;
#[cfg(any(test, feature = "testing"))]
pub mod fixtures;
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
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::files::transport::FileTransport;
use operations::{DispatchDeps, Operation, Outcome};

pub struct CallLogging<'state, 'env> {
    pub state: &'state mut LoggingState,
    pub env: &'state PrePostEnv<'env>,
}

#[derive(Debug, thiserror::Error)]
pub enum DispatchError<Failure: std::error::Error + Send + Sync + 'static> {
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
    log: &'call mut CallLogging<'_, '_>,
) -> completion::HandlerFuture<'call, Outcome<Handler::Reply>, DispatchError<Handler::Failure>>
where
    Client: 'call,
    Encoding: 'call,
    Transport: CountTokensTransport + BatchTransport + FileTransport,
    Handler: completion::CompletionHandler<Client, Encoding>,
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
