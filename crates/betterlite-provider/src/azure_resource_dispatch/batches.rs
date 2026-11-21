use std::future::Future;
use std::pin::Pin;

use betterlite_azure::batches::error::AzureBatchesError;
use betterlite_azure::batches::{Operation, Reply, call};
use betterlite_openai::batches::records::BatchPage;
use betterlite_openai::batches::sdk::BatchesSdk;

pub struct Dispatch<'call, C: BatchesSdk<Page = BatchPage>> {
    pub client: &'call C,
    pub operation: Operation<'call>,
}

pub type Outcome = Reply;

pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, AzureBatchesError>> + Send + 'call>>;

impl<C: BatchesSdk<Page = BatchPage>> Dispatch<'_, C> {
    // B541's four lifecycle calls, each one sent through the SDK client the caller supplied
    pub async fn run(self) -> Result<Outcome, AzureBatchesError> {
        call(self.client, self.operation).await
    }
}

pub fn dispatch<'call, C>(dispatch: Dispatch<'call, C>) -> RunFuture<'call>
where
    C: BatchesSdk<Page = BatchPage> + 'call,
{
    Box::pin(dispatch.run())
}
