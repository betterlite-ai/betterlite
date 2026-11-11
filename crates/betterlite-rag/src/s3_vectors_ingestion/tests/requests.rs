use std::collections::BTreeMap;

use rstest::rstest;

use crate::s3_vectors_ingestion::tests::support::{
    FakeTransport, REGION, RecordedBucketBody, RecordedCreateIndexBody, RecordedIndexBody,
    RecordedPutVectorsBody, body_hash, failure, header, rag_clients, response, response_with_body,
};
use crate::s3_vectors_ingestion::{
    S3VectorData, S3VectorEntry, S3VectorsError, S3VectorsRequests, VectorIndexConfig,
};

const GET_BUCKET_URL: &str = "https://s3vectors.us-west-2.api.aws/GetVectorBucket";
const CREATE_BUCKET_URL: &str = "https://s3vectors.us-west-2.api.aws/CreateVectorBucket";
const GET_INDEX_URL: &str = "https://s3vectors.us-west-2.api.aws/GetIndex";
const CREATE_INDEX_URL: &str = "https://s3vectors.us-west-2.api.aws/CreateIndex";
const PUT_VECTORS_URL: &str = "https://s3vectors.us-west-2.api.aws/PutVectors";

fn entry(key: &str) -> S3VectorEntry {
    S3VectorEntry {
        key: key.to_owned(),
        data: S3VectorData {
            float32: vec![0.5, 0.25],
        },
        metadata: BTreeMap::from([("source_text".to_owned(), "hello".to_owned())]),
    }
}

fn index_config(keys: &[String]) -> VectorIndexConfig<'_> {
    VectorIndexConfig {
        bucket_name: "my-vector-bucket",
        index_name: "my-index",
        dimension: 1024,
        distance_metric: "cosine",
        non_filterable_metadata_keys: keys,
    }
}

#[tokio::test]
async fn signed_request_carries_the_body_hash() {
    let transport = FakeTransport::new(vec![response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_bucket_exists("my-vector-bucket")
        .await
        .expect("the bucket exists");
    let recorded = transport.requests();
    let request = recorded.first().expect("one GetVectorBucket request");
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, GET_BUCKET_URL);
    assert_eq!(header(request, "Content-Type"), Some("application/json"));
    let expected_hash = body_hash(&request.body);
    assert_eq!(
        header(request, "x-amz-content-sha256"),
        Some(expected_hash.as_str())
    );
    let authorization = header(request, "Authorization").expect("the request is signed");
    assert!(authorization.contains("/us-west-2/s3vectors/aws4_request"));
}

#[tokio::test]
async fn signed_request_uses_the_resolved_region() {
    let transport = FakeTransport::new(vec![response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: "eu-central-1",
    };
    requests
        .execute("GetIndex", b"{}")
        .await
        .expect("the empty payload is signed");
    let recorded = transport.requests();
    let request = recorded.first().expect("one signed request");
    assert_eq!(
        request.url,
        "https://s3vectors.eu-central-1.api.aws/GetIndex"
    );
    let authorization = header(request, "Authorization").expect("the request is signed");
    assert!(authorization.contains("/eu-central-1/s3vectors/aws4_request"));
}

#[tokio::test]
async fn signed_request_hashes_an_empty_body() {
    let transport = FakeTransport::new(vec![response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .execute("GetVectorBucket", &[])
        .await
        .expect("the empty body is signed");
    let recorded = transport.requests();
    let request = recorded.first().expect("one signed request");
    assert_eq!(header(request, "Content-Type"), None);
    assert_eq!(
        header(request, "x-amz-content-sha256"),
        Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    );
}

#[tokio::test]
async fn bucket_exists_get_then_create_on_not_found() {
    let transport = FakeTransport::new(vec![response(404), response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_bucket_exists("my-vector-bucket")
        .await
        .expect("the bucket is created");
    let recorded = transport.requests();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].url, GET_BUCKET_URL);
    assert_eq!(recorded[1].url, CREATE_BUCKET_URL);
    assert_eq!(recorded[0].body, recorded[1].body);
    let body: RecordedBucketBody =
        serde_json::from_slice(&recorded[1].body).expect("the bucket body");
    assert_eq!(body.vector_bucket_name, "my-vector-bucket");
}

#[tokio::test]
async fn bucket_exists_creates_after_a_failed_get() {
    let transport = FakeTransport::new(vec![failure(), response(201)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_bucket_exists("my-vector-bucket")
        .await
        .expect("the bucket is created");
    let recorded = transport.requests();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[1].url, CREATE_BUCKET_URL);
}

#[tokio::test]
async fn bucket_exists_accepts_409_from_create() {
    let transport = FakeTransport::new(vec![response(403), response(409)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_bucket_exists("my-vector-bucket")
        .await
        .expect("an existing bucket is not an error");
    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test]
async fn bucket_exists_returns_on_the_first_get_without_creating() {
    let transport = FakeTransport::new(vec![response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_bucket_exists("my-vector-bucket")
        .await
        .expect("the bucket exists");
    let recorded = transport.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, GET_BUCKET_URL);
}

#[tokio::test]
async fn bucket_exists_creates_when_the_get_is_not_ok() {
    let transport = FakeTransport::new(vec![response(201), response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_bucket_exists("my-vector-bucket")
        .await
        .expect("the bucket is created");
    let recorded = transport.requests();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[1].url, CREATE_BUCKET_URL);
}

#[tokio::test]
async fn bucket_exists_fails_on_a_create_status_it_does_not_accept() {
    let transport = FakeTransport::new(vec![response(404), response_with_body(500, "boom")]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    let error = requests
        .ensure_vector_bucket_exists("my-vector-bucket")
        .await
        .expect_err("a failed creation is an error");
    let message = error.to_string();
    assert!(message.contains("CreateVectorBucket"));
    assert!(message.contains("500"));
    assert!(message.contains("boom"));
}

#[rstest]
#[case::one_character("a")]
#[case::too_short("ab")]
#[case::underscore("bad_bucket")]
#[case::only_separators("...")]
#[tokio::test]
async fn bucket_names_are_validated_before_a_request(#[case] bucket_name: &str) {
    let transport = FakeTransport::new(vec![response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    let error = requests
        .ensure_vector_bucket_exists(bucket_name)
        .await
        .expect_err("the bucket name is rejected");
    assert!(matches!(error, S3VectorsError::Input(_)));
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn bucket_names_at_the_minimum_length_reach_the_api() {
    let transport = FakeTransport::new(vec![response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_bucket_exists("abc")
        .await
        .expect("the bucket exists");
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn bucket_names_with_dots_and_hyphens_reach_the_api() {
    let transport = FakeTransport::new(vec![response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_bucket_exists("my.vector-bucket")
        .await
        .expect("the bucket exists");
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn index_exists_get_then_create_on_not_found() {
    let keys = vec!["source_text".to_owned()];
    let transport = FakeTransport::new(vec![response(404), response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_index_exists(&index_config(&keys))
        .await
        .expect("the index is created");
    let recorded = transport.requests();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].url, GET_INDEX_URL);
    assert_eq!(recorded[1].url, CREATE_INDEX_URL);
    let body: RecordedIndexBody =
        serde_json::from_slice(&recorded[0].body).expect("the index body");
    assert_eq!(body.vector_bucket_name, "my-vector-bucket");
    assert_eq!(body.index_name, "my-index");
}

#[tokio::test]
async fn index_exists_creates_after_a_failed_get() {
    let keys = vec!["source_text".to_owned()];
    let transport = FakeTransport::new(vec![failure(), response(201)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_index_exists(&index_config(&keys))
        .await
        .expect("the index is created");
    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test]
async fn index_exists_accepts_409_from_create() {
    let keys = vec!["source_text".to_owned()];
    let transport = FakeTransport::new(vec![response(404), response(409)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_index_exists(&index_config(&keys))
        .await
        .expect("an existing index is not an error");
    assert_eq!(transport.requests().len(), 2);
}

#[tokio::test]
async fn index_exists_returns_on_200_without_creating() {
    let keys = vec!["source_text".to_owned()];
    let transport = FakeTransport::new(vec![response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_index_exists(&index_config(&keys))
        .await
        .expect("the index exists");
    let recorded = transport.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].url, GET_INDEX_URL);
}

#[tokio::test]
async fn index_exists_sends_the_caller_dimension_and_metric() {
    let keys = vec!["source_text".to_owned(), "filename".to_owned()];
    let transport = FakeTransport::new(vec![response(404), response(201)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_index_exists(&index_config(&keys))
        .await
        .expect("the index is created");
    let recorded = transport.requests();
    let body: RecordedCreateIndexBody =
        serde_json::from_slice(&recorded[1].body).expect("the create index body");
    assert_eq!(body.vector_bucket_name, "my-vector-bucket");
    assert_eq!(body.index_name, "my-index");
    assert_eq!(body.data_type, "float32");
    assert_eq!(body.dimension, 1024);
    assert_eq!(body.distance_metric, "cosine");
    let metadata = body.metadata_configuration.expect("the metadata keys");
    assert_eq!(
        metadata.non_filterable_metadata_keys,
        vec!["source_text".to_owned(), "filename".to_owned()]
    );
}

#[tokio::test]
async fn index_exists_omits_the_metadata_configuration_when_empty() {
    let keys: Vec<String> = Vec::new();
    let transport = FakeTransport::new(vec![response(404), response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_index_exists(&index_config(&keys))
        .await
        .expect("the index is created");
    let recorded = transport.requests();
    let body: RecordedCreateIndexBody =
        serde_json::from_slice(&recorded[1].body).expect("the create index body");
    assert!(body.metadata_configuration.is_none());
}

#[tokio::test]
async fn index_exists_creates_when_the_get_is_not_ok() {
    let keys = vec!["source_text".to_owned()];
    let transport = FakeTransport::new(vec![response(201), response(200)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .ensure_vector_index_exists(&index_config(&keys))
        .await
        .expect("the index is created");
    let recorded = transport.requests();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[1].url, CREATE_INDEX_URL);
}

#[tokio::test]
async fn index_exists_fails_on_a_create_status_it_does_not_accept() {
    let keys = vec!["source_text".to_owned()];
    let transport = FakeTransport::new(vec![response(404), response_with_body(500, "boom")]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    let error = requests
        .ensure_vector_index_exists(&index_config(&keys))
        .await
        .expect_err("a failed creation is an error");
    let message = error.to_string();
    assert!(message.contains("CreateIndex"));
    assert!(message.contains("500"));
}

#[tokio::test]
async fn put_vectors_posts_the_entries() {
    let entries = vec![entry("doc_0"), entry("doc_1")];
    let transport = FakeTransport::new(vec![response(201)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .put_vectors("my-vector-bucket", "my-index", &entries)
        .await
        .expect("the vectors are stored");
    let recorded = transport.requests();
    let request = recorded.first().expect("one PutVectors request");
    assert_eq!(request.url, PUT_VECTORS_URL);
    let body: RecordedPutVectorsBody =
        serde_json::from_slice(&request.body).expect("the put vectors body");
    assert_eq!(body.vector_bucket_name, "my-vector-bucket");
    assert_eq!(body.index_name, "my-index");
    assert_eq!(body.vectors, entries);
    let expected_hash = body_hash(&request.body);
    assert_eq!(
        header(request, "x-amz-content-sha256"),
        Some(expected_hash.as_str())
    );
}

#[rstest]
#[case::ok(200)]
#[case::created(201)]
#[tokio::test]
async fn put_vectors_accepts_the_success_statuses(#[case] status: u16) {
    let transport = FakeTransport::new(vec![response(status)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    requests
        .put_vectors("my-vector-bucket", "my-index", &[entry("doc_0")])
        .await
        .expect("the vectors are stored");
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn put_vectors_reports_a_transport_failure() {
    let transport = FakeTransport::new(vec![failure()]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    let error = requests
        .put_vectors("my-vector-bucket", "my-index", &[entry("doc_0")])
        .await
        .expect_err("a failed transport is an error");
    assert!(matches!(error, S3VectorsError::Request(_)));
}

#[tokio::test]
async fn put_vectors_fails_on_409() {
    let transport = FakeTransport::new(vec![response(409)]);
    let clients = rag_clients(transport.clone());
    let requests = S3VectorsRequests {
        clients: &clients,
        aws_region_name: REGION,
    };
    let error = requests
        .put_vectors("my-vector-bucket", "my-index", &[entry("doc_0")])
        .await
        .expect_err("a conflict is an error for PutVectors");
    assert!(error.to_string().contains("PutVectors"));
}
