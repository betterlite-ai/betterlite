#![cfg(test)]

use std::fs;
use std::io::Error;
use std::path::PathBuf;

use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::batches::BatchOperation;
use crate::batches::decode::{BatchDecodeError, MAX_BATCH_BYTES, decode_batch_object};
use crate::batches::error::BatchesError;
use crate::batches::records::{BatchStatus, ExpiryAnchor, OutputExpiry};
use crate::batches::sdk::{ClientKind, Timeout};
use crate::batches::tests::fake::{
    Answer, Recorded, batch, batch_with, connection, create_request, failure, fixture, page, query,
    supplied,
};

mod fake;
mod http;

const NOT_INITIALIZED: &str = "OpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment.";
const NOT_ASYNC: &str =
    "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client.";
const NOT_SYNC: &str =
    "OpenAI client is not an instance of OpenAI. Make sure you passed a sync OpenAI client.";

fn pairs(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(key, value)| (String::from(*key), String::from(*value)))
        .collect()
}

async fn create_window_endpoint() {
    let fixture = fixture(true, Answer::Batch(batch("batch_1", "validating")));
    let mut request = create_request();
    request.metadata = Some(pairs(&[("team", "ml")]));
    request.extra_body = Some(pairs(&[("vendor", "x")]));
    request.output_expires_after = Some(OutputExpiry {
        anchor: ExpiryAnchor::CreatedAt,
        seconds: 3600,
    });
    let created = fixture
        .api
        .batch(connection(false, None), BatchOperation::Create(&request))
        .await
        .unwrap();
    assert_eq!(created.id, "batch_1");
    assert_eq!(created.completion_window, "24h");
    assert_eq!(created.endpoint, "/v1/embeddings");
    assert_eq!(*fixture.calls.lock().unwrap(), [Recorded::Create(request)]);
}

async fn metadata() {
    let extra = r#","metadata":{"team":"ml"},"usage":{"input_tokens":3,"output_tokens":4,"total_tokens":7}"#;
    let fixture = fixture(
        true,
        Answer::Batch(batch_with("batch_1", "validating", extra)),
    );
    let found = fixture
        .api
        .batch(connection(false, None), BatchOperation::Retrieve(&query()))
        .await
        .unwrap();
    assert_eq!(found.metadata, Some(pairs(&[("team", "ml")])));
    let usage = found.usage.unwrap();
    let counts = (
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.total_tokens,
    );
    assert_eq!(counts, (0_i64, 0_i64, 7_i64));
    assert_eq!(usage.sdk_fields["input_tokens"], 3_i64);
    let bare = self::fixture(true, Answer::Batch(batch("batch_2", "validating")));
    let created = bare
        .api
        .batch(
            connection(false, None),
            BatchOperation::Create(&create_request()),
        )
        .await
        .unwrap();
    assert_eq!((created.metadata, created.usage), (None, None));
}

async fn retrieve_status_counts() {
    let extra = r#","request_counts":{"completed":3,"failed":1,"total":5}"#;
    let answer = Answer::Batch(batch_with("batch_1", "in_progress", extra));
    let fixture = fixture(true, answer);
    let found = fixture
        .api
        .batch(connection(true, None), BatchOperation::Retrieve(&query()))
        .await
        .unwrap();
    assert_eq!(found.status, BatchStatus::InProgress);
    let counts = found.request_counts.unwrap();
    assert_eq!(
        (counts.completed, counts.failed, counts.total),
        (3_i64, 1_i64, 5_i64)
    );
    assert_eq!(
        *fixture.calls.lock().unwrap(),
        [Recorded::Retrieve(query())]
    );
}

async fn cancel_batch() {
    let fixture = fixture(true, Answer::Batch(batch("batch_1", "cancelling")));
    let cancelled = fixture
        .api
        .batch(connection(true, None), BatchOperation::Cancel(&query()))
        .await
        .unwrap();
    assert_eq!(cancelled.status, BatchStatus::Cancelling);
    assert_eq!(*fixture.calls.lock().unwrap(), [Recorded::Cancel(query())]);
}

async fn list_after_limit() {
    let fixture = fixture(true, Answer::Batch(batch("batch_1", "completed")));
    let by_default = fixture
        .api
        .list_batches(connection(false, None), None, None)
        .await
        .unwrap();
    let with_values = fixture
        .api
        .list_batches(connection(true, None), Some("batch_0"), Some(20))
        .await
        .unwrap();
    assert_eq!((by_default, with_values), (page(), page()));
    let first = Recorded::List {
        after: None,
        limit: None,
    };
    let second = Recorded::List {
        after: Some(String::from("batch_0")),
        limit: Some(20),
    };
    assert_eq!(*fixture.calls.lock().unwrap(), [first, second]);
}

async fn batch_error() {
    let extra = r#","errors":{"object":"list","data":[{"code":"invalid_json_line","line":4,"message":"bad line","param":null}]}"#;
    let fixture = fixture(true, Answer::Batch(batch_with("batch_1", "failed", extra)));
    let found = fixture
        .api
        .batch(connection(false, None), BatchOperation::Retrieve(&query()))
        .await
        .unwrap();
    let failures = found.errors.unwrap().data.unwrap();
    assert_eq!(failures[0].code.as_deref(), Some("invalid_json_line"));
    assert_eq!(failures[0].line, Some(4_i64));
    let unknown = self::fixture(true, Answer::Batch(batch("batch_1", "exploded")));
    let error = unknown
        .api
        .batch(connection(false, None), BatchOperation::Retrieve(&query()))
        .await
        .unwrap_err();
    assert!(matches!(error, BatchesError::Invalid { .. }));
}

async fn sdk_client_argument_projection() {
    let fixture = fixture(true, Answer::Batch(batch("batch_1", "validating")));
    let lookup = query();
    let retrieve = |connection| {
        fixture
            .api
            .batch(connection, BatchOperation::Retrieve(&lookup))
    };
    let mut asynchronous = connection(true, None);
    asynchronous.settings.api_key = Some(SecretString::from(""));
    asynchronous.settings.api_base = Some(String::from("https://example.test/v1"));
    asynchronous.settings.max_retries = Some(0);
    asynchronous.settings.organization = Some(String::new());
    retrieve(asynchronous).await.unwrap();
    let mut synchronous = connection(false, None);
    synchronous.settings.timeout = Some(Timeout::Seconds(30.0));
    retrieve(synchronous).await.unwrap();
    let answer = Answer::Batch(batch("batch_1", "validating"));
    let client = supplied(&fixture, ClientKind::Sync, answer);
    let own = connection(false, Some(client));
    retrieve(own).await.unwrap();
    let built = fixture.built.lock().unwrap().clone();
    assert_eq!(built.len(), 2);
    let (first, first_kind) = &built[0];
    assert_eq!(*first_kind, ClientKind::Async);
    assert_eq!(first.api_key.as_ref().unwrap().expose_secret(), "");
    assert_eq!(first.base_url.as_deref(), Some("https://example.test/v1"));
    assert_eq!(first.max_retries, Some(0));
    assert_eq!(first.organization.as_deref(), Some(""));
    assert_eq!(first.timeout, None);
    let (second, second_kind) = &built[1];
    assert_eq!(*second_kind, ClientKind::Sync);
    assert!(second.api_key.is_none() && second.base_url.is_none());
    assert!(second.max_retries.is_none() && second.organization.is_none());
    assert_eq!(second.timeout, Some(Timeout::Seconds(30.0)));
}

async fn sdk_client_missing() {
    let fixture = fixture(false, Answer::Batch(batch("batch_1", "validating")));
    let error = fixture
        .api
        .batch(
            connection(false, None),
            BatchOperation::Create(&create_request()),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, BatchesError::NotInitialized));
    assert_eq!(error.to_string(), NOT_INITIALIZED);
    let listing = fixture
        .api
        .list_batches(connection(false, None), None, None)
        .await;
    assert!(matches!(listing, Err(BatchesError::NotInitialized)));
}

async fn sdk_client_wrong_async() {
    let fixture = fixture(true, Answer::Batch(batch("batch_1", "validating")));
    let sync_client = supplied(&fixture, ClientKind::Sync, Answer::Failure);
    let error = fixture
        .api
        .batch(
            connection(true, Some(sync_client)),
            BatchOperation::Retrieve(&query()),
        )
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), NOT_ASYNC);
    assert!(fixture.calls.lock().unwrap().is_empty());
}

async fn sdk_error_passthrough() {
    let fixture = fixture(true, Answer::Failure);
    let api = &fixture.api;
    let errors = [
        api.batch(
            connection(false, None),
            BatchOperation::Create(&create_request()),
        )
        .await
        .unwrap_err(),
        api.batch(connection(false, None), BatchOperation::Retrieve(&query()))
            .await
            .unwrap_err(),
        api.batch(connection(false, None), BatchOperation::Cancel(&query()))
            .await
            .unwrap_err(),
        api.list_batches(connection(false, None), None, None)
            .await
            .map(|_| ())
            .unwrap_err(),
    ];
    for error in errors {
        let BatchesError::Sdk { source } = error else {
            panic!("expected the SDK error to pass through");
        };
        assert_eq!(format!("{source:?}"), format!("{:?}", failure()));
    }
}

async fn cancel_sync_client_type() {
    let fixture = fixture(true, Answer::Batch(batch("batch_1", "validating")));
    let wrong = || {
        let answer = Answer::Batch(batch("batch_1", "validating"));
        connection(false, Some(supplied(&fixture, ClientKind::Async, answer)))
    };
    let error = fixture
        .api
        .batch(wrong(), BatchOperation::Cancel(&query()))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), NOT_SYNC);
    assert!(fixture.calls.lock().unwrap().is_empty());
    fixture
        .api
        .batch(wrong(), BatchOperation::Retrieve(&query()))
        .await
        .unwrap();
    fixture
        .api
        .batch(wrong(), BatchOperation::Create(&create_request()))
        .await
        .unwrap();
    fixture.api.list_batches(wrong(), None, None).await.unwrap();
    assert_eq!(fixture.calls.lock().unwrap().len(), 3);
}

#[derive(Debug, Clone, Copy)]
enum Scenario {
    CreateWindowEndpoint,
    Metadata,
    RetrieveStatusCounts,
    CancelBatch,
    ListAfterLimit,
    BatchError,
    SdkClientArgumentProjection,
    SdkClientMissing,
    SdkClientWrongAsync,
    SdkErrorPassthrough,
    CancelSyncClientType,
}

#[rstest]
#[case::create_window_endpoint(Scenario::CreateWindowEndpoint)]
#[case::metadata(Scenario::Metadata)]
#[case::retrieve_status_counts(Scenario::RetrieveStatusCounts)]
#[case::cancel_batch(Scenario::CancelBatch)]
#[case::list_after_limit(Scenario::ListAfterLimit)]
#[case::batch_error(Scenario::BatchError)]
#[case::sdk_client_argument_projection(Scenario::SdkClientArgumentProjection)]
#[case::sdk_client_missing(Scenario::SdkClientMissing)]
#[case::sdk_client_wrong_async(Scenario::SdkClientWrongAsync)]
#[case::sdk_error_passthrough(Scenario::SdkErrorPassthrough)]
#[case::cancel_sync_client_type(Scenario::CancelSyncClientType)]
#[tokio::test]
async fn batches_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::CreateWindowEndpoint => create_window_endpoint().await,
        Scenario::Metadata => metadata().await,
        Scenario::RetrieveStatusCounts => retrieve_status_counts().await,
        Scenario::CancelBatch => cancel_batch().await,
        Scenario::ListAfterLimit => list_after_limit().await,
        Scenario::BatchError => batch_error().await,
        Scenario::SdkClientArgumentProjection => sdk_client_argument_projection().await,
        Scenario::SdkClientMissing => sdk_client_missing().await,
        Scenario::SdkClientWrongAsync => sdk_client_wrong_async().await,
        Scenario::SdkErrorPassthrough => sdk_error_passthrough().await,
        Scenario::CancelSyncClientType => cancel_sync_client_type().await,
    }
}

#[test]
fn batch_answers_over_the_byte_limit_are_refused() {
    let at_limit = vec![b' '; MAX_BATCH_BYTES];
    assert!(matches!(
        decode_batch_object(&at_limit),
        Err(BatchDecodeError::Json { .. })
    ));
    let over_limit = vec![b' '; MAX_BATCH_BYTES + 1];
    assert!(matches!(
        decode_batch_object(&over_limit),
        Err(BatchDecodeError::TooLarge { size }) if size == MAX_BATCH_BYTES + 1
    ));
}

#[test]
fn replays_the_openai_batches_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_batches");
    let mut paths = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "no batch corpus seeds");
    paths
        .into_iter()
        .try_for_each(|path| {
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let decoded = decode_batch_object(&bytes);
            if name.starts_with("valid-") {
                assert!(decoded.is_ok(), "{name}: {decoded:?}");
            } else {
                assert!(
                    matches!(decoded, Err(BatchDecodeError::Json { .. })),
                    "{name}: {decoded:?}"
                );
            }
            Ok::<(), Error>(())
        })
        .unwrap();
}
