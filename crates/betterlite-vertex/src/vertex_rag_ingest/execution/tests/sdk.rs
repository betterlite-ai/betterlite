use std::future::{Future, ready};
use std::sync::{Arc, Mutex};

use google_cloud_aiplatform_v1::client::VertexRagDataService;
use google_cloud_aiplatform_v1::model::import_rag_files_config::ImportSource;
use google_cloud_aiplatform_v1::model::rag_file_chunking_config::ChunkingConfig as WireChunkingConfig;
use google_cloud_aiplatform_v1::model::{ImportRagFilesRequest, ImportRagFilesResponse};
use google_cloud_aiplatform_v1::stub::VertexRagDataService as VertexRagDataStub;
use google_cloud_auth::credentials::testing::error_credentials;
use google_cloud_gax::error::Error;
use google_cloud_gax::options::RequestOptions;
use google_cloud_gax::response::Response;
use google_cloud_longrunning::model::GetOperationRequest;
use google_cloud_longrunning::model::Operation;
use google_cloud_longrunning::model::operation::Result as OperationResult;
use google_cloud_wkt::Any;
use rstest::rstest;

use crate::vertex_rag_ingest::chunking::{
    ChunkDefaults, ChunkingConfig, ChunkingRecord, RagChunkingStrategy, transform_chunking_strategy,
};
use crate::vertex_rag_ingest::ports::{CorpusImport, ImportRequest};
use crate::vertex_rag_ingest::sdk::VertexRagDataImport;

const CORPUS_NAME: &str = "projects/project/locations/us-central1/ragCorpora/corpus";
const GCS_URI: &str = "gs://bucket/report.txt";

#[derive(Debug, Clone, Default)]
struct CaptureImport {
    requests: Arc<Mutex<Vec<ImportRagFilesRequest>>>,
    operations: Arc<Mutex<Vec<GetOperationRequest>>>,
    completed_on_poll: bool,
}

impl VertexRagDataStub for CaptureImport {
    fn import_rag_files(
        &self,
        request: ImportRagFilesRequest,
        _options: RequestOptions,
    ) -> impl Future<Output = Result<Response<Operation>, Error>> + Send {
        self.requests
            .lock()
            .expect("the capture lock")
            .push(request);
        let operation = if self.completed_on_poll {
            pending_operation()
        } else {
            done_operation(3)
        };
        ready(Ok(Response::from(operation)))
    }

    fn get_operation(
        &self,
        request: GetOperationRequest,
        _options: RequestOptions,
    ) -> impl Future<Output = Result<Response<Operation>, Error>> + Send {
        self.operations
            .lock()
            .expect("the capture lock")
            .push(request);
        ready(Ok(Response::from(done_operation(7))))
    }
}

fn done_operation(imported_rag_files_count: i64) -> Operation {
    let response =
        ImportRagFilesResponse::new().set_imported_rag_files_count(imported_rag_files_count);
    let result = Any::from_msg(&response).expect("the response becomes an Any");
    Operation::new()
        .set_name("operations/import")
        .set_done(true)
        .set_result(OperationResult::Response(Box::new(result)))
}

fn pending_operation() -> Operation {
    Operation::new().set_name("operations/import")
}

fn import_request(chunking: ChunkingRecord) -> ImportRequest {
    ImportRequest {
        corpus: CORPUS_NAME.to_owned(),
        gcs_uri: GCS_URI.to_owned(),
        chunking,
    }
}

fn fixed_length(chunk_size: i64, chunk_overlap: i64) -> ChunkingRecord {
    ChunkingRecord {
        chunking_config: ChunkingConfig {
            chunk_size,
            chunk_overlap,
        },
    }
}

#[test]
fn the_adapter_takes_an_endpoint_and_optional_credentials() {
    let adc = VertexRagDataImport::new("https://aiplatform.googleapis.com", None);
    assert!(matches!(adc, VertexRagDataImport::Adc { .. }));
    let explicit = VertexRagDataImport::new(
        "https://aiplatform.googleapis.com",
        Some(error_credentials(false)),
    );
    assert!(matches!(explicit, VertexRagDataImport::Credentials { .. }));
}

#[tokio::test]
async fn import_request_carries_the_chunking_config() {
    let capture = CaptureImport::default();
    let port = VertexRagDataImport::Existing(VertexRagDataService::from_stub(capture.clone()));
    let strategy: RagChunkingStrategy =
        serde_json::from_str(r#"{"chunk_size": 512, "chunk_overlap": 16}"#)
            .expect("the strategy is valid");
    let chunking = transform_chunking_strategy(Some(&strategy), ChunkDefaults::LITELLM);
    let handle = port
        .submit(import_request(chunking))
        .await
        .expect("the stubbed import starts");
    let outcome = port
        .wait(handle)
        .await
        .expect("the stubbed import completes");
    assert_eq!(outcome.imported_rag_files_count, 3);
    let requests = capture.requests.lock().expect("the capture lock");
    let request = requests.first().expect("the SDK sent one request");
    assert_eq!(request.parent, CORPUS_NAME);
    let config = request
        .import_rag_files_config
        .as_ref()
        .expect("the request carries the config");
    let source = match config
        .import_source
        .as_ref()
        .expect("the import source is set")
    {
        ImportSource::GcsSource(source) => source,
        other => panic!("the import source is a GCS source, got {other:?}"),
    };
    assert_eq!(source.uris, vec![GCS_URI.to_owned()]);
    let wire_chunking = config
        .rag_file_transformation_config
        .as_ref()
        .expect("the transformation config is set")
        .rag_file_chunking_config
        .as_ref()
        .expect("the chunking config is set")
        .chunking_config
        .as_ref()
        .expect("the chunking entry is set");
    let WireChunkingConfig::FixedLengthChunking(fixed) = wire_chunking else {
        panic!("the chunking config is fixed length");
    };
    assert_eq!(fixed.chunk_size, 512);
    assert_eq!(fixed.chunk_overlap, 16);
}

#[rstest]
#[case::chunk_size(i64::MAX, 0, "chunk_size")]
#[case::chunk_overlap(0, i64::MAX, "chunk_overlap")]
#[tokio::test]
async fn an_out_of_range_chunk_value_is_refused(
    #[case] chunk_size: i64,
    #[case] chunk_overlap: i64,
    #[case] field: &str,
) {
    let capture = CaptureImport::default();
    let port = VertexRagDataImport::Existing(VertexRagDataService::from_stub(capture.clone()));
    let error = port
        .submit(import_request(fixed_length(chunk_size, chunk_overlap)))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("the {field} value 9223372036854775807 does not fit the Vertex RAG integer field")
    );
    assert!(
        capture
            .requests
            .lock()
            .expect("the capture lock")
            .is_empty()
    );
}

#[tokio::test(start_paused = true)]
async fn wait_polls_the_submitted_operation() {
    let capture = CaptureImport {
        completed_on_poll: true,
        ..CaptureImport::default()
    };
    let port = VertexRagDataImport::Existing(VertexRagDataService::from_stub(capture.clone()));
    let handle = port
        .submit(import_request(fixed_length(10, 1)))
        .await
        .expect("the stubbed import starts");
    let outcome = port
        .wait(handle)
        .await
        .expect("the polled import completes");
    assert_eq!(outcome.imported_rag_files_count, 7);
    assert_eq!(
        capture.requests.lock().expect("the capture lock").len(),
        1,
        "the wait must not submit the import again"
    );
    let operations = capture.operations.lock().expect("the capture lock");
    assert_eq!(operations.len(), 1);
    assert_eq!(operations[0].name, "operations/import");
}
