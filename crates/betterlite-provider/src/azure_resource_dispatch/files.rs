use std::future::Future;
use std::pin::Pin;

use betterlite_azure::azure_files::Reply;
use betterlite_azure::azure_files::error::AzureFilesError;

pub type Outcome = Reply;

// B694's five lifecycle calls finish in the leaf, so the caller boxes the future this dispatch awaits
pub type RunFuture<'call> =
    Pin<Box<dyn Future<Output = Result<Outcome, AzureFilesError>> + Send + 'call>>;
