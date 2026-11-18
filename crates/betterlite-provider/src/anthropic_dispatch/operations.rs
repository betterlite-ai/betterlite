use betterlite_anthropic::batches::handler::{AnthropicBatchesHandler, RetrieveRequest};
use betterlite_anthropic::count_tokens::handler::AnthropicCountTokensHandler;
use betterlite_anthropic::count_tokens::wire::{CountResponse, CountTokensRequest};
use betterlite_anthropic::files::api::FilesApi;
use betterlite_anthropic::files::request::{FileCall, FilesClient, ListCall, Upload};
use betterlite_openai::batches::records::BatchObject;
use betterlite_openai::files::records::{BinaryContent, FileDeleted, FileObject};

use super::base::Surface;
use super::completion::{CompletionCall, CompletionSettings};
use super::config::AnthropicConfigKind;

pub enum Operation<'call, Client, Encoding> {
    Completion(Surface, Box<CompletionCall<Client, Encoding>>),
    Count(CountTokensRequest<'call>),
    BatchCreate,
    BatchRetrieve(RetrieveRequest<'call>),
    BatchResults(FileCall<'call>),
    Upload(Upload<'call>),
    Retrieve(FileCall<'call>),
    List(ListCall<'call>),
    Delete(FileCall<'call>),
    Content(FileCall<'call>),
    ConfigUrl(&'call str),
}

pub enum Outcome<Reply> {
    Completion(Reply),
    Count(CountResponse),
    Batch(Box<BatchObject>),
    File(Box<FileObject>),
    Files(Vec<FileObject>),
    Deleted(FileDeleted),
    Content(BinaryContent),
    Config(AnthropicConfigKind),
}

pub struct DispatchDeps<'deps, Transport, Handler> {
    pub completion: &'deps CompletionSettings<'deps>,
    pub handler: &'deps mut Handler,
    pub count: &'deps AnthropicCountTokensHandler<Transport>,
    pub batches: &'deps AnthropicBatchesHandler<Transport>,
    pub files: &'deps FilesApi<Transport>,
    pub file_client: &'deps FilesClient,
}
