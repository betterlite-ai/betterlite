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
    let operation = OpenAiResourceOperation::batches(Dispatch {
        api,
        connection,
        call,
    });
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

async fn create_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let api = api(&calls, BatchAnswer::Body(batch_body("batch_created")));
    let data = create_data();
    let result = run(&api, Call::Create(&data)).await;
    let Ok(OpenAiResourceResult::Batches(Outcome::Batch(batch))) = result else {
        panic!("the create reply decodes to a batch");
    };
    assert_eq!(batch.id, "batch_created");
    assert_eq!(batch.endpoint, "/v1/embeddings");
    assert_eq!(batch.completion_window, "24h");
    let recorded = calls.lock().unwrap();
    let BatchCall::Create(sent) = &recorded[0] else {
        panic!("the create call reaches the SDK");
    };
    assert_eq!(sent.completion_window, CompletionWindow::Day);
    assert_eq!(sent.endpoint, BatchEndpoint::Embeddings);
    assert_eq!(sent.input_file_id, "file-1");
    assert_eq!(
        sent.output_expires_after
            .as_ref()
            .map(|expiry| expiry.seconds),
        Some(3600)
    );
}

async fn retrieve_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let api = api(&calls, BatchAnswer::Body(batch_body("batch_retrieved")));
    let data = query();
    let result = run(&api, Call::Retrieve(&data)).await;
    let Ok(OpenAiResourceResult::Batches(Outcome::Batch(batch))) = result else {
        panic!("the retrieve reply decodes to a batch");
    };
    assert_eq!(batch.id, "batch_retrieved");
    let recorded = calls.lock().unwrap();
    assert!(matches!(&recorded[0], BatchCall::Retrieve(sent) if sent.batch_id == "batch_1"));
}

async fn cancel_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let api = api(&calls, BatchAnswer::Body(batch_body("batch_cancelled")));
    let data = query();
    let result = run(&api, Call::Cancel(&data)).await;
    let Ok(OpenAiResourceResult::Batches(Outcome::Batch(batch))) = result else {
        panic!("the cancel reply decodes to a batch");
    };
    assert_eq!(batch.id, "batch_cancelled");
    let recorded = calls.lock().unwrap();
    assert!(matches!(&recorded[0], BatchCall::Cancel(sent) if sent.batch_id == "batch_1"));
}

async fn list_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let api = api(&calls, BatchAnswer::Body(batch_body("batch_listed")));
    let result = run(
        &api,
        Call::List {
            after: Some("batch_0"),
            limit: Some(5),
        },
    )
    .await;
    let Ok(OpenAiResourceResult::Batches(Outcome::Page(page))) = result else {
        panic!("the list reply is the SDK page");
    };
    assert_eq!(page, batch_page());
    let recorded = calls.lock().unwrap();
    let BatchCall::List { after, limit } = &recorded[0] else {
        panic!("the list call reaches the SDK");
    };
    assert_eq!(after.as_deref(), Some("batch_0"));
    assert_eq!(*limit, Some(5));
}

async fn error_row() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let api = api(&calls, BatchAnswer::Failure);
    let data = query();
    let result = run(&api, Call::Retrieve(&data)).await;
    let Err(error) = result else {
        panic!("the SDK status error is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::Batches(BatchesError::Sdk(_))
    ));
}

pub(crate) async fn check() {
    create_row().await;
    retrieve_row().await;
    cancel_row().await;
    list_row().await;
    error_row().await;
}
