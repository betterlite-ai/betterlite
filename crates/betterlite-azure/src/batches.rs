pub mod client;
pub mod error;

use betterlite_openai::batches::decode::decode_batch_object;
use betterlite_openai::batches::records::{BatchCreate, BatchObject, BatchPage, BatchQuery};
use betterlite_openai::batches::sdk::BatchesSdk;

use self::client::ClientCheck;
use self::error::AzureBatchesError;

// The call a caller asks for; list carries the cursor pair instead of a request record
#[derive(Debug, Clone, Copy)]
pub enum Operation<'request> {
    Create(&'request BatchCreate),
    Retrieve(&'request BatchQuery),
    Cancel(&'request BatchQuery),
    List {
        after: Option<&'request str>,
        limit: Option<i64>,
    },
}

impl Operation<'_> {
    // Which class the open step checks; cancel names both async classes
    pub const fn check(&self) -> ClientCheck {
        match self {
            Self::Cancel(_) => ClientCheck::Cancel,
            Self::Create(_) | Self::Retrieve(_) | Self::List { .. } => ClientCheck::Lifecycle,
        }
    }
}

// Create, retrieve and cancel answer with a batch; list answers with the SDK's own page
#[derive(Debug)]
pub enum Reply {
    Batch(Box<BatchObject>),
    Page(Box<BatchPage>),
}

// LiteLLM forwards the request record as it is and validates the SDK's model_dump into LiteLLMBatch
pub async fn call<C>(client: &C, operation: Operation<'_>) -> Result<Reply, AzureBatchesError>
where
    C: BatchesSdk<Page = BatchPage>,
{
    match operation {
        Operation::Create(request) => Ok(Reply::Batch(Box::new(decode_batch_object(
            &client.create(request).await?,
        )?))),
        Operation::Retrieve(request) => Ok(Reply::Batch(Box::new(decode_batch_object(
            &client.retrieve(request).await?,
        )?))),
        Operation::Cancel(request) => Ok(Reply::Batch(Box::new(decode_batch_object(
            &client.cancel(request).await?,
        )?))),
        Operation::List { after, limit } => {
            Ok(Reply::Page(Box::new(client.list(after, limit).await?)))
        }
    }
}

mod tests;
