use std::future::Future;
use std::pin::Pin;

use betterlite_azure::batches::Reply;
use betterlite_azure::batches::error::AzureBatchesError;

pub type Outcome = Reply;

// B541's four lifecycle calls finish in the leaf, so the caller boxes the future this dispatch awaits
pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, AzureBatchesError>> + Send + 'call>>;
