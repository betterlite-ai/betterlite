pub mod error;
pub mod service;

mod tests;

use betterlite_models::decisions::Json;

use self::error::VectorFileReadError;
use self::service::{ReadSession, entry_from_flag, read_attachment};
use crate::api::vector_file_http_read::service::{Operation, ReadTransport};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource};

// either the file object or content reply, or the failure B011 maps to the SDK exception it raises
pub type ReadResult = Result<Json, VectorFileReadError>;

// The read session's four operations, each picking the handler and the wrapper flag it pops
impl<S> ReadSession<'_, '_, '_, '_, S>
where
    S: ClientSource<Client: ReadTransport>,
{
    // retrieve: only a literal true selects the asynchronous handler
    pub async fn retrieve(&mut self) -> ReadResult {
        let flag = "aretrieve";
        let mode = entry_from_flag(self.call.kwargs, flag);
        read_attachment(self, Operation::Retrieve, mode, flag).await
    }

    // aretrieve: LiteLLM sets the flag itself, so the retrieve path always takes the asynchronous handler
    pub async fn aretrieve(&mut self) -> ReadResult {
        let flag = "aretrieve";
        let mode = ClientKind::Asynchronous;
        read_attachment(self, Operation::Retrieve, mode, flag).await
    }

    // retrieve_content: the content wrapper's own flag is the one it pops
    pub async fn retrieve_content(&mut self) -> ReadResult {
        let flag = "aretrieve_content";
        let mode = entry_from_flag(self.call.kwargs, flag);
        read_attachment(self, Operation::Content, mode, flag).await
    }

    // aretrieve_content: LiteLLM sets the flag itself, so the content path always takes the asynchronous handler
    pub async fn aretrieve_content(&mut self) -> ReadResult {
        let flag = "aretrieve_content";
        let mode = ClientKind::Asynchronous;
        read_attachment(self, Operation::Content, mode, flag).await
    }
}
