use betterlite_openai::batches::error::BatchesError;
use betterlite_openai::batches::records::{BatchCreate, BatchObject, BatchPage, BatchQuery};
use betterlite_openai::batches::sdk::{BatchesSdk, ClientConstructor, Connection};
use betterlite_openai::batches::{BatchOperation, BatchesApi};

pub struct Dispatch<'call, C>
where
    C: ClientConstructor,
    C::Client: BatchesSdk<Page = BatchPage>,
{
    pub api: &'call BatchesApi<C>,
    pub connection: Connection<C::Client>,
    pub call: Call<'call>,
}

// B517's four lifecycle calls; list keeps the cursor and limit it was given
pub enum Call<'call> {
    Create(&'call BatchCreate),
    Retrieve(&'call BatchQuery),
    Cancel(&'call BatchQuery),
    List {
        after: Option<&'call str>,
        limit: Option<i64>,
    },
}

#[derive(Debug)]
pub enum Outcome {
    Batch(Box<BatchObject>),
    Page(Box<BatchPage>),
}

impl<C: ClientConstructor> Dispatch<'_, C>
where
    C::Client: BatchesSdk<Page = BatchPage>,
{
    pub async fn run(self) -> Result<Outcome, BatchesError> {
        let Self {
            api,
            connection,
            call,
        } = self;
        Ok(match call {
            Call::Create(request) => Outcome::Batch(Box::new(
                api.batch(connection, BatchOperation::Create(request))
                    .await?,
            )),
            Call::Retrieve(request) => Outcome::Batch(Box::new(
                api.batch(connection, BatchOperation::Retrieve(request))
                    .await?,
            )),
            Call::Cancel(request) => Outcome::Batch(Box::new(
                api.batch(connection, BatchOperation::Cancel(request))
                    .await?,
            )),
            Call::List { after, limit } => {
                Outcome::Page(Box::new(api.list_batches(connection, after, limit).await?))
            }
        })
    }
}
