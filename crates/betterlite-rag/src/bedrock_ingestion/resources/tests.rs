#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use tokio::time::Instant;
use uuid::Uuid;

use crate::bedrock_ingestion::client::{AwsRagResponse, BedrockRagClients};
use crate::bedrock_ingestion::error::BedrockIngestionError;
use crate::bedrock_ingestion::state::BedrockRagState;
use crate::bedrock_ingestion::tests::support::{
    FakeTransport, RecordedRequest, StaticResolver, clients, json_response, settings,
    status_response,
};
use crate::bedrock_ingestion::wire::{
    CreateDataSourceRequest, CreateKnowledgeBaseRequest, DataSourceType, StorageType,
};

const LIST_ONE: &str = r#"{"dataSourceSummaries":[{"dataSourceId":"DS1"}]}"#;
const LIST_TWO: &str = r#"{"dataSourceSummaries":[{"dataSourceId":"DS1"},{"dataSourceId":"DS2"}]}"#;
const LIST_EMPTY: &str = r#"{"dataSourceSummaries":[]}"#;
const DETAIL_BUCKET: &str = r#"{"dataSource":{"dataSourceConfiguration":{"s3Configuration":{"bucketArn":"arn:aws:s3:::docs-bucket"}}}}"#;
const DETAIL_PREFIXES: &str = r#"{"dataSource":{"dataSourceConfiguration":{"s3Configuration":{"bucketArn":"arn:aws:s3:::docs-bucket","inclusionPrefixes":["kb/docs/","other/"]}}}}"#;
const DETAIL_EMPTY_ARN: &str =
    r#"{"dataSource":{"dataSourceConfiguration":{"s3Configuration":{"bucketArn":""}}}}"#;
const DETAIL_NO_CONFIG: &str = r#"{"dataSource":{"dataSourceId":"DS1"}}"#;
const DETAIL_NULL_CONFIG: &str = r#"{"dataSource":{"dataSourceConfiguration":null}}"#;
const DETAIL_NULL_S3: &str =
    r#"{"dataSource":{"dataSourceConfiguration":{"s3Configuration":null}}}"#;
const KB_CREATE: &str = r#"{"knowledgeBase":{"knowledgeBaseId":"KB-NEW"}}"#;
const DS_CREATE: &str = r#"{"dataSource":{"dataSourceId":"DS-NEW"}}"#;
const EMPTY_BODY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const US_WEST_2_BODY_SHA256: &str =
    "a4a3d263179bfcbaf88755b14ea9db3048e4179c03bbc94ccf0f13c8c2d3df6a";
const US_WEST_2_BODY: &str = "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><LocationConstraint>us-west-2</LocationConstraint></CreateBucketConfiguration>";

fn setup(
    vector: &str,
    responses: Vec<AwsRagResponse>,
) -> (
    BedrockRagClients<'static, FakeTransport>,
    FakeTransport,
    BedrockRagState,
) {
    let transport = FakeTransport::new(responses);
    let resolver = StaticResolver::new(None);
    let parsed = settings(vector);
    let state = BedrockRagState::new(&parsed);
    let built = clients(parsed, &resolver, transport.clone());
    (built, transport, state)
}

fn kb_setup(
    vector: &str,
    statuses: &[&str],
) -> (
    BedrockRagClients<'static, FakeTransport>,
    FakeTransport,
    BedrockRagState,
) {
    let mut responses = vec![json_response(KB_CREATE)];
    for status in statuses {
        responses.push(json_response(&format!(
            r#"{{"knowledgeBase":{{"status":"{status}"}}}}"#
        )));
    }
    setup(vector, responses)
}

fn source_setup(
    vector: &str,
    bucket: Option<&str>,
) -> (
    BedrockRagClients<'static, FakeTransport>,
    FakeTransport,
    BedrockRagState,
) {
    let (built, transport, mut state) = setup(vector, vec![json_response(DS_CREATE)]);
    state.s3_bucket = bucket.map(str::to_owned);
    (built, transport, state)
}

fn header<'request>(request: &'request RecordedRequest, name: &str) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[tokio::test]
async fn detect_uses_the_first_data_source_from_the_list() {
    let (built, _transport, mut state) = setup(
        r#"{"vector_store_id": "KB123"}"#,
        vec![json_response(LIST_TWO), json_response(DETAIL_BUCKET)],
    );
    state
        .auto_detect_config(&built)
        .await
        .expect("the data source is detected");
    assert_eq!(state.data_source_id.as_deref(), Some("DS1"));
    assert_eq!(state.s3_bucket.as_deref(), Some("docs-bucket"));
    assert_eq!(state.s3_prefix, "data/");
}

#[tokio::test]
async fn detect_configured_data_source_wins() {
    let (built, transport, mut state) = setup(
        r#"{"vector_store_id": "KB123", "data_source_id": "DS9"}"#,
        vec![json_response(LIST_ONE), json_response(DETAIL_BUCKET)],
    );
    state
        .auto_detect_config(&built)
        .await
        .expect("the data source is detected");
    assert_eq!(state.data_source_id.as_deref(), Some("DS9"));
    let requests = transport.requests();
    assert_eq!(
        requests[1].url,
        "https://bedrock-agent.us-west-2.amazonaws.com/knowledgebases/KB123/datasources/DS9"
    );
}

#[tokio::test]
async fn detect_configured_bucket_wins_over_the_arn_bucket() {
    let (built, _transport, mut state) = setup(
        r#"{"vector_store_id": "KB123", "s3_bucket": "configured-bucket"}"#,
        vec![json_response(LIST_ONE), json_response(DETAIL_BUCKET)],
    );
    state
        .auto_detect_config(&built)
        .await
        .expect("the data source is detected");
    assert_eq!(state.s3_bucket.as_deref(), Some("configured-bucket"));
}

#[tokio::test]
async fn detect_uses_the_first_inclusion_prefix_when_none_is_configured() {
    let (built, _transport, mut state) = setup(
        r#"{"vector_store_id": "KB123"}"#,
        vec![json_response(LIST_ONE), json_response(DETAIL_PREFIXES)],
    );
    state
        .auto_detect_config(&built)
        .await
        .expect("the data source is detected");
    assert_eq!(state.s3_prefix, "kb/docs/");
}

#[tokio::test]
async fn detect_configured_prefix_blocks_the_inclusion_prefix() {
    let (built, _transport, mut state) = setup(
        r#"{"vector_store_id": "KB123", "s3_prefix": "configured/"}"#,
        vec![json_response(LIST_ONE), json_response(DETAIL_PREFIXES)],
    );
    state
        .auto_detect_config(&built)
        .await
        .expect("the data source is detected");
    assert_eq!(state.s3_prefix, "configured/");
}

#[tokio::test]
async fn detect_missing_s3_configuration_falls_back_to_the_configured_bucket() {
    let (built, _transport, mut state) = setup(
        r#"{"vector_store_id": "KB123", "s3_bucket": "configured-bucket"}"#,
        vec![json_response(LIST_ONE), json_response(DETAIL_NO_CONFIG)],
    );
    state
        .auto_detect_config(&built)
        .await
        .expect("the data source is detected");
    assert_eq!(state.s3_bucket.as_deref(), Some("configured-bucket"));
    assert_eq!(state.s3_prefix, "data/");
}

#[rstest]
#[case::null_configuration(DETAIL_NULL_CONFIG)]
#[case::null_s3_configuration(DETAIL_NULL_S3)]
#[tokio::test]
async fn detect_null_configuration_falls_back_to_the_configured_bucket(#[case] detail: &str) {
    let (built, _transport, mut state) = setup(
        r#"{"vector_store_id": "KB123", "s3_bucket": "configured-bucket"}"#,
        vec![json_response(LIST_ONE), json_response(detail)],
    );
    state
        .auto_detect_config(&built)
        .await
        .expect("the configured bucket is used");
    assert_eq!(state.s3_bucket.as_deref(), Some("configured-bucket"));
    assert_eq!(state.s3_prefix, "data/");
}

#[rstest]
#[case::null_configuration(DETAIL_NULL_CONFIG)]
#[case::null_s3_configuration(DETAIL_NULL_S3)]
#[tokio::test]
async fn detect_null_configuration_without_a_bucket_names_litellms_message(#[case] detail: &str) {
    let (built, _transport, mut state) = setup(
        r#"{"vector_store_id": "KB123"}"#,
        vec![json_response(LIST_ONE), json_response(detail)],
    );
    let error = state
        .auto_detect_config(&built)
        .await
        .expect_err("there is no bucket");
    assert_eq!(
        error.to_string(),
        "Could not auto-detect S3 bucket for data source DS1. Please provide s3_bucket in config."
    );
}

#[tokio::test]
async fn detect_no_data_sources_names_litellms_message() {
    let (built, _transport, mut state) = setup(
        r#"{"vector_store_id": "KB123"}"#,
        vec![json_response(LIST_EMPTY)],
    );
    let error = state
        .auto_detect_config(&built)
        .await
        .expect_err("there is no data source");
    assert_eq!(
        error.to_string(),
        "No data sources found for Knowledge Base KB123. Please create a data source first or provide data_source_id and s3_bucket."
    );
}

#[tokio::test]
async fn detect_empty_bucket_arn_names_litellms_message() {
    let (built, _transport, mut state) = setup(
        r#"{"vector_store_id": "KB123"}"#,
        vec![json_response(LIST_ONE), json_response(DETAIL_EMPTY_ARN)],
    );
    let error = state
        .auto_detect_config(&built)
        .await
        .expect_err("there is no bucket");
    assert_eq!(
        error.to_string(),
        "Could not auto-detect S3 bucket for data source DS1. Please provide s3_bucket in config."
    );
}

#[tokio::test]
async fn detect_missing_knowledge_base_id_is_a_typed_error() {
    let (built, _transport, mut state) = setup("{}", vec![json_response(LIST_EMPTY)]);
    let error = state
        .auto_detect_config(&built)
        .await
        .expect_err("there is no knowledge base");
    assert!(
        matches!(error, BedrockIngestionError::MissingKnowledgeBaseId),
        "{error:?}"
    );
}

#[tokio::test]
async fn detect_reads_only_the_first_page() {
    let (built, transport, mut state) = setup(
        r#"{"vector_store_id": "KB123"}"#,
        vec![json_response(LIST_ONE), json_response(DETAIL_BUCKET)],
    );
    state
        .auto_detect_config(&built)
        .await
        .expect("the data source is detected");
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url,
        "https://bedrock-agent.us-west-2.amazonaws.com/knowledgebases/KB123/datasources/"
    );
}

#[tokio::test]
async fn bucket_names_and_records_the_new_bucket() {
    let (built, _transport, mut state) = setup("{}", vec![json_response("")]);
    let name = state
        .create_s3_bucket(&built, "abcd1234")
        .await
        .expect("the bucket is created");
    assert_eq!(name, "litellm-kb-abcd1234");
    assert_eq!(
        state.created_resources.s3_bucket.as_deref(),
        Some("litellm-kb-abcd1234")
    );
}

#[tokio::test]
async fn bucket_sends_the_location_constraint_outside_us_east_1() {
    let (built, transport, mut state) = setup("{}", vec![json_response("")]);
    state
        .create_s3_bucket(&built, "abcd1234")
        .await
        .expect("the bucket is created");
    let requests = transport.requests();
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(
        requests[0].url,
        "https://litellm-kb-abcd1234.s3.us-west-2.amazonaws.com/"
    );
    assert_eq!(
        String::from_utf8(requests[0].body.clone()).expect("the XML body is UTF-8"),
        US_WEST_2_BODY
    );
    assert_eq!(
        header(&requests[0], "x-amz-content-sha256"),
        Some(US_WEST_2_BODY_SHA256)
    );
}

#[tokio::test]
async fn bucket_sends_no_body_in_us_east_1() {
    let (built, transport, mut state) = setup(
        r#"{"aws_region_name": "us-east-1"}"#,
        vec![json_response("")],
    );
    state
        .create_s3_bucket(&built, "abcd1234")
        .await
        .expect("the bucket is created");
    let requests = transport.requests();
    assert_eq!(
        requests[0].url,
        "https://litellm-kb-abcd1234.s3.amazonaws.com/"
    );
    assert!(requests[0].body.is_empty());
    assert_eq!(
        header(&requests[0], "x-amz-content-sha256"),
        Some(EMPTY_BODY_SHA256)
    );
}

#[tokio::test]
async fn bucket_uses_the_partition_endpoint_and_region() {
    let (built, transport, mut state) = setup(
        r#"{"aws_region_name": "cn-north-1"}"#,
        vec![json_response("")],
    );
    state
        .create_s3_bucket(&built, "abcd1234")
        .await
        .expect("the bucket is created");
    let requests = transport.requests();
    assert_eq!(
        requests[0].url,
        "https://litellm-kb-abcd1234.s3.cn-north-1.amazonaws.com.cn/"
    );
    let authorization = header(&requests[0], "Authorization").expect("the request is signed");
    assert!(
        authorization.contains("/cn-north-1/s3/aws4_request"),
        "{authorization}"
    );
}

#[tokio::test]
async fn bucket_non_2xx_is_a_create_bucket_error() {
    let (built, _transport, mut state) =
        setup("{}", vec![status_response(400, "InvalidBucketName")]);
    let error = state
        .create_s3_bucket(&built, "abcd1234")
        .await
        .expect_err("S3 rejects the request");
    let BedrockIngestionError::Aws {
        operation, status, ..
    } = &error
    else {
        panic!("expected the AWS error, got {error:?}");
    };
    assert_eq!(*operation, "CreateBucket");
    assert_eq!(*status, 400);
}

#[tokio::test]
async fn kb_create_request_matches_litellm() {
    let (built, transport, mut state) = kb_setup("{}", &["ACTIVE"]);
    state
        .create_knowledge_base(&built, "litellm-kb-abcd1234", "arn:role", "arn:collection")
        .await
        .expect("the knowledge base is created");
    let requests = transport.requests();
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(
        requests[0].url,
        "https://bedrock-agent.us-west-2.amazonaws.com/knowledgebases/"
    );
    let body: CreateKnowledgeBaseRequest =
        serde_json::from_slice(&requests[0].body).expect("the create body");
    assert_eq!(body.name, "litellm-kb-abcd1234");
    assert_eq!(body.role_arn, "arn:role");
    let configuration = body.knowledge_base_configuration;
    assert_eq!(
        configuration.vector.embedding_model_arn,
        "arn:aws:bedrock:us-west-2::foundation-model/amazon.titan-embed-text-v2:0"
    );
    let storage = body.storage_configuration;
    assert_eq!(storage.storage_type, StorageType::OpensearchServerless);
    let serverless = storage.aoss;
    assert_eq!(serverless.collection_arn, "arn:collection");
    assert_eq!(serverless.vector_index_name, "bedrock-kb-index");
    assert_eq!(
        serverless.field_mapping.metadata_field,
        "AMAZON_BEDROCK_METADATA"
    );
    assert_eq!(
        serverless.field_mapping.text_field,
        "AMAZON_BEDROCK_TEXT_CHUNK"
    );
    assert_eq!(
        serverless.field_mapping.vector_field,
        "bedrock-knowledge-base-default-vector"
    );
}

#[tokio::test]
async fn kb_client_token_is_a_fresh_v4_uuid() {
    let active = r#"{"knowledgeBase":{"status":"ACTIVE"}}"#;
    let (built, transport, mut state) = setup(
        "{}",
        vec![
            json_response(KB_CREATE),
            json_response(active),
            json_response(KB_CREATE),
            json_response(active),
        ],
    );
    state
        .create_knowledge_base(&built, "kb", "arn:role", "arn:collection")
        .await
        .expect("the knowledge base is created");
    state
        .create_knowledge_base(&built, "kb", "arn:role", "arn:collection")
        .await
        .expect("the knowledge base is created again");
    let requests = transport.requests();
    let first: CreateKnowledgeBaseRequest =
        serde_json::from_slice(&requests[0].body).expect("the create body");
    let second: CreateKnowledgeBaseRequest =
        serde_json::from_slice(&requests[2].body).expect("the create body");
    assert_eq!(
        Uuid::parse_str(&first.client_token)
            .expect("a uuid")
            .get_version_num(),
        4
    );
    assert_ne!(first.client_token, second.client_token);
}

#[tokio::test]
async fn kb_returns_the_created_id_when_active() {
    let (built, _transport, mut state) = kb_setup("{}", &["ACTIVE"]);
    let knowledge_base_id = state
        .create_knowledge_base(&built, "kb", "arn:role", "arn:collection")
        .await
        .expect("the knowledge base is created");
    assert_eq!(knowledge_base_id, "KB-NEW");
    assert_eq!(
        state.created_resources.knowledge_base.as_deref(),
        Some("KB-NEW")
    );
}

#[tokio::test(start_paused = true)]
async fn kb_stops_without_sleeping_when_active() {
    let (built, _transport, mut state) = kb_setup("{}", &["ACTIVE"]);
    let start = Instant::now();
    state
        .create_knowledge_base(&built, "kb", "arn:role", "arn:collection")
        .await
        .expect("the knowledge base is created");
    assert_eq!(start.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn kb_waits_for_active_after_inactive_checks() {
    let (built, transport, mut state) = kb_setup("{}", &["CREATING", "CREATING", "ACTIVE"]);
    let start = Instant::now();
    state
        .create_knowledge_base(&built, "kb", "arn:role", "arn:collection")
        .await
        .expect("the knowledge base is created");
    assert_eq!(start.elapsed(), Duration::from_secs(4));
    assert_eq!(transport.requests().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn kb_times_out_after_thirty_checks_and_sixty_seconds() {
    let statuses = vec!["CREATING"; 30];
    let (built, transport, mut state) = kb_setup("{}", &statuses);
    let start = Instant::now();
    let error = state
        .create_knowledge_base(&built, "kb", "arn:role", "arn:collection")
        .await
        .expect_err("the knowledge base never becomes active");
    assert!(
        matches!(error, BedrockIngestionError::KnowledgeBaseTimeout),
        "{error:?}"
    );
    assert_eq!(start.elapsed(), Duration::from_secs(60));
    assert_eq!(transport.requests().len(), 31);
    assert_eq!(
        state.created_resources.knowledge_base.as_deref(),
        Some("KB-NEW")
    );
}

#[tokio::test]
async fn kb_uses_the_partition_arn_prefix_and_endpoint() {
    let (built, transport, mut state) =
        kb_setup(r#"{"aws_region_name": "cn-north-1"}"#, &["ACTIVE"]);
    state
        .create_knowledge_base(&built, "kb", "arn:role", "arn:collection")
        .await
        .expect("the knowledge base is created");
    let requests = transport.requests();
    assert_eq!(
        requests[0].url,
        "https://bedrock-agent.cn-north-1.amazonaws.com.cn/knowledgebases/"
    );
    let body: CreateKnowledgeBaseRequest =
        serde_json::from_slice(&requests[0].body).expect("the create body");
    assert_eq!(
        body.knowledge_base_configuration.vector.embedding_model_arn,
        "arn:aws-cn:bedrock:cn-north-1::foundation-model/amazon.titan-embed-text-v2:0"
    );
}

#[tokio::test]
async fn kb_uses_the_configured_embedding_model() {
    let (built, transport, mut state) = kb_setup(
        r#"{"embedding_model": "cohere.embed-english-v3"}"#,
        &["ACTIVE"],
    );
    state
        .create_knowledge_base(&built, "kb", "arn:role", "arn:collection")
        .await
        .expect("the knowledge base is created");
    let body: CreateKnowledgeBaseRequest =
        serde_json::from_slice(&transport.requests()[0].body).expect("the create body");
    assert_eq!(
        body.knowledge_base_configuration.vector.embedding_model_arn,
        "arn:aws:bedrock:us-west-2::foundation-model/cohere.embed-english-v3"
    );
}

#[tokio::test]
async fn kb_non_2xx_create_is_an_error() {
    let (built, _transport, mut state) =
        setup("{}", vec![status_response(400, "ValidationException")]);
    let error = state
        .create_knowledge_base(&built, "kb", "arn:role", "arn:collection")
        .await
        .expect_err("the create call fails");
    let BedrockIngestionError::Aws {
        operation, status, ..
    } = &error
    else {
        panic!("expected the AWS error, got {error:?}");
    };
    assert_eq!(*operation, "CreateKnowledgeBase");
    assert_eq!(*status, 400);
    assert!(state.created_resources.knowledge_base.is_none());
}

#[tokio::test]
async fn source_sends_the_s3_configuration() {
    let (built, transport, mut state) = source_setup(
        r#"{"vector_store_id": "KB-CONFIG", "s3_prefix": "docs/"}"#,
        Some("bucket"),
    );
    let data_source_id = state
        .create_data_source(&built, "litellm-kb-abcd1234")
        .await
        .expect("the data source is created");
    assert_eq!(data_source_id, "DS-NEW");
    let requests = transport.requests();
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(
        requests[0].url,
        "https://bedrock-agent.us-west-2.amazonaws.com/knowledgebases/KB-CONFIG/datasources/"
    );
    let body: CreateDataSourceRequest =
        serde_json::from_slice(&requests[0].body).expect("the create body");
    assert_eq!(body.name, "litellm-kb-abcd1234-s3-source");
    assert_eq!(
        body.data_source_configuration.data_source_type,
        DataSourceType::S3
    );
    assert_eq!(
        body.data_source_configuration.s3_configuration.bucket_arn,
        "arn:aws:s3:::bucket"
    );
    assert_eq!(
        body.data_source_configuration
            .s3_configuration
            .inclusion_prefixes,
        vec!["docs/".to_owned()]
    );
    assert_eq!(
        Uuid::parse_str(&body.client_token)
            .expect("a uuid")
            .get_version_num(),
        4
    );
}

#[tokio::test]
async fn source_records_the_created_data_source() {
    let (built, _transport, mut state) =
        source_setup(r#"{"vector_store_id": "KB-CONFIG"}"#, Some("bucket"));
    state
        .create_data_source(&built, "kb")
        .await
        .expect("the data source is created");
    assert_eq!(
        state.created_resources.data_source.as_deref(),
        Some("DS-NEW")
    );
}

#[tokio::test]
async fn source_uses_the_created_knowledge_base_over_the_configured_one() {
    let (built, transport, mut state) =
        source_setup(r#"{"vector_store_id": "KB-CONFIG"}"#, Some("bucket"));
    state.created_resources.knowledge_base = Some("KB-CREATED".to_owned());
    state
        .create_data_source(&built, "kb")
        .await
        .expect("the data source is created");
    let requests = transport.requests();
    assert_eq!(
        requests[0].url,
        "https://bedrock-agent.us-west-2.amazonaws.com/knowledgebases/KB-CREATED/datasources/"
    );
    let body: CreateDataSourceRequest =
        serde_json::from_slice(&requests[0].body).expect("the create body");
    assert_eq!(body.name, "kb-s3-source");
}

#[tokio::test]
async fn source_uses_the_partition_arn_prefix() {
    let (built, transport, mut state) = source_setup(
        r#"{"vector_store_id": "KB", "aws_region_name": "cn-north-1"}"#,
        Some("bucket"),
    );
    state
        .create_data_source(&built, "kb")
        .await
        .expect("the data source is created");
    let body: CreateDataSourceRequest =
        serde_json::from_slice(&transport.requests()[0].body).expect("the create body");
    assert_eq!(
        body.data_source_configuration.s3_configuration.bucket_arn,
        "arn:aws-cn:s3:::bucket"
    );
}

#[tokio::test]
async fn source_requires_a_bucket() {
    let (built, _transport, mut state) = source_setup(r#"{"vector_store_id": "KB-CONFIG"}"#, None);
    let error = state
        .create_data_source(&built, "kb")
        .await
        .expect_err("there is no bucket");
    assert!(
        matches!(error, BedrockIngestionError::MissingS3Bucket),
        "{error:?}"
    );
}

#[tokio::test]
async fn source_requires_a_knowledge_base() {
    let (built, _transport, mut state) = source_setup("{}", Some("bucket"));
    let error = state
        .create_data_source(&built, "kb")
        .await
        .expect_err("there is no knowledge base");
    assert!(
        matches!(error, BedrockIngestionError::MissingKnowledgeBaseId),
        "{error:?}"
    );
}

#[tokio::test]
async fn source_non_2xx_is_an_error() {
    let transport = FakeTransport::new(vec![status_response(400, "ValidationException")]);
    let resolver = StaticResolver::new(None);
    let parsed = settings(r#"{"vector_store_id": "KB-CONFIG"}"#);
    let mut state = BedrockRagState::new(&parsed);
    state.s3_bucket = Some("bucket".to_owned());
    let built = clients(parsed, &resolver, transport);
    let error = state
        .create_data_source(&built, "kb")
        .await
        .expect_err("the create call fails");
    let BedrockIngestionError::Aws {
        operation, status, ..
    } = &error
    else {
        panic!("expected the AWS error, got {error:?}");
    };
    assert_eq!(*operation, "CreateDataSource");
    assert_eq!(*status, 400);
    assert!(state.created_resources.data_source.is_none());
}
