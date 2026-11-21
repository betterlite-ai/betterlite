use std::future::Future;
use std::pin::Pin;

use betterlite_azure::azure_files::client::Opened;
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{Operation, Reply, call};
use betterlite_openai::files::transport::FileTransport;

pub struct Dispatch<'call, T: FileTransport> {
    pub opened: &'call Opened,
    pub transport: &'call T,
    pub operation: Operation<'call>,
}

pub type Outcome = Reply;

pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, AzureFilesError>> + Send + 'call>>;

impl<T: FileTransport> Dispatch<'_, T> {
    // B694's five lifecycle calls, each one sent through the client the connection opened
    pub async fn run(self) -> Result<Outcome, AzureFilesError> {
        call(self.opened, self.transport, self.operation).await
    }
}

pub fn dispatch<'call, T>(dispatch: Dispatch<'call, T>) -> RunFuture<'call>
where
    T: FileTransport + 'call,
{
    Box::pin(dispatch.run())
}
