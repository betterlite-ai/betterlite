use std::sync::{Arc, Mutex};

use betterlite_openai::batches::BatchesApi;
use betterlite_openai::batches::error::BatchesError;
use betterlite_openai::batches::records::{
    BatchCreate, BatchEndpoint, BatchQuery, CompletionWindow, ExpiryAnchor, OutputExpiry,
};
use betterlite_openai::batches::sdk::Connection;
use indexmap::IndexMap;

use super::fixture::{
    BatchAnswer, BatchCall, BatchConstructor, batch_body, batch_page, batch_settings,
};
use crate::openai_resource_dispatch::batches::{Call, Dispatch, Outcome};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

fn api(calls: &Arc<Mutex<Vec<BatchCall>>>, answer: BatchAnswer) -> BatchesApi<BatchConstructor> {
    BatchesApi {
        constructor: BatchConstructor {
            calls: Arc::clone(calls),
            answer,
        },
    }
}

async fn run(
    api: &BatchesApi<BatchConstructor>,
    call: Call<'_>,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let connection = Connection {
        is_async: true,
        settings: batch_settings(),
        client: None,
    };
    let operation = OpenAiResourceOperation::Batches(Box::pin(
        Dispatch {
            api,
            connection,
            call,
        }
        .run(),
    ));
    dispatch(operation).await
}

fn create_data() -> BatchCreate {
    BatchCreate {
        completion_window: CompletionWindow::Day,
        endpoint: BatchEndpoint::Embeddings,
        input_file_id: String::from("file-1"),
        metadata: Some(IndexMap::from([(
            String::from("team"),
            String::from("core"),
        )])),
        output_expires_after: Some(OutputExpiry {
            anchor: ExpiryAnchor::CreatedAt,
            seconds: 3600,
        }),
        extra_headers: None,
        extra_body: None,
        timeout: Some(30.0),
    }
}

fn query() -> BatchQuery {
    BatchQuery {
        batch_id: String::from("batch_1"),
        extra_headers: None,
        extra_body: None,
        timeout: None,
    }
}

async fn batch_row(call: Call<'_>, id: &str, sent: BatchCall) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let api = api(&calls, BatchAnswer::Body(batch_body(id)));
    let result = run(&api, call).await;
    let Ok(OpenAiResourceResult::Batches(Outcome::Batch(batch))) = result else {
        panic!("the reply decodes to a batch");
    };
    assert_eq!(batch.id, id);
    assert_eq!(batch.endpoint, "/v1/embeddings");
    assert_eq!(batch.completion_window, "24h");
    assert_eq!(*calls.lock().unwrap(), [sent]);
}

async fn list_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let api = api(&calls, BatchAnswer::Body(batch_body("batch_listed")));
    let call = Call::List {
        after: Some("batch_0"),
        limit: Some(5),
    };
    let result = run(&api, call).await;
    let Ok(OpenAiResourceResult::Batches(Outcome::Page(page))) = result else {
        panic!("the list reply is the SDK page");
    };
    assert_eq!(*page, batch_page());
    let listed = BatchCall::List {
        after: Some(String::from("batch_0")),
        limit: Some(5),
    };
    assert_eq!(*calls.lock().unwrap(), [listed]);
}

async fn error_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let api = api(&calls, BatchAnswer::Failure);
    let lookup = query();
    let result = run(&api, Call::Retrieve(&lookup)).await;
    let Err(error) = result else {
        panic!("the SDK status error is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Batches(BatchesError::Sdk { .. })
    ));
}

pub(crate) async fn check() {
    let data = create_data();
    let lookup = query();
    let created = BatchCall::Create(data.clone());
    batch_row(Call::Create(&data), "batch_created", created).await;
    let retrieved = BatchCall::Retrieve(lookup.clone());
    batch_row(Call::Retrieve(&lookup), "batch_retrieved", retrieved).await;
    let cancelled = BatchCall::Cancel(lookup.clone());
    batch_row(Call::Cancel(&lookup), "batch_cancelled", cancelled).await;
    list_row().await;
    error_row().await;
}
