#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use aws_smithy_async::rt::sleep::{SharedAsyncSleep, TokioSleep};
use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use aws_smithy_runtime_api::client::http::{
    HttpConnector, HttpConnectorFuture, SharedHttpConnector, http_client_fn,
};
use aws_smithy_runtime_api::client::orchestrator::HttpRequest;
use aws_smithy_runtime_api::client::result::ConnectorError;
use aws_types::os_shim_internal::Fs;
use betterlite_bedrock::aws_endpoints::sts::StsClientKwargs;
use betterlite_bedrock::aws_settings::credential_cache::CredentialCache;
use betterlite_bedrock::aws_settings::sources::NativeSources;
use betterlite_bedrock::credentials::AwsCredentialResolver;
use betterlite_config::tls::Verification;
use betterlite_context::local_ttl_cache::LocalTtlCache;
use betterlite_context::local_ttl_cache::dual::{CacheEvent, Diagnostics, DualLocalCache};
use betterlite_redact::secret::Redactor;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::{ExposeSecret, SecretString};

use crate::bedrock_ingestion::client::clock::{Clock as _, SystemClock};
use crate::bedrock_ingestion::client::credentials::RagCredentialResolver as _;
use crate::bedrock_ingestion::client::endpoints::{
    bedrock_agent_endpoint, create_bucket_body, s3_bucket_host,
};
use crate::bedrock_ingestion::client::{
    AwsRagResponse, BedrockRagClients, SignedRequest, aws_settings,
};
use crate::bedrock_ingestion::error::BedrockIngestionError;
use crate::bedrock_ingestion::tests::support::{
    FakeTransport, FakeTransportError, NOW_SECONDS, RecordedRequest, StaticResolver, TEST_KEY,
    clients, fixed_now, json_response, settings, status_response,
};
use crate::bedrock_ingestion::wire::{
    AossConfiguration, AossFieldMapping, CreateDataSourceRequest, CreateKnowledgeBaseRequest,
    DataSourceType, KnowledgeBaseConfiguration, KnowledgeBaseType, NewDataSourceConfiguration,
    NewS3DataSourceConfiguration, StorageConfiguration, StorageType, VectorConfiguration,
};

const AGENT: &str = "https://bedrock-agent.us-west-2.amazonaws.com";

fn header<'request>(request: &'request RecordedRequest, name: &str) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn secret(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

fn rag_clients(
    responses: Vec<AwsRagResponse>,
) -> (
    BedrockRagClients<'static, FakeTransport>,
    FakeTransport,
    StaticResolver,
) {
    let transport = FakeTransport::new(responses);
    let resolver = StaticResolver::new(None);
    let parsed = settings(r#"{"vector_store_id": "KB123", "aws_region_name": "us-west-2"}"#);
    let built = clients(parsed, &resolver, transport.clone());
    (built, transport, resolver)
}

fn knowledge_base_request() -> CreateKnowledgeBaseRequest {
    CreateKnowledgeBaseRequest {
        client_token: "token".to_owned(),
        name: "kb".to_owned(),
        role_arn: "arn:aws:iam::123456789012:role/Role".to_owned(),
        knowledge_base_configuration: KnowledgeBaseConfiguration {
            knowledge_base_type: KnowledgeBaseType::Vector,
            vector: VectorConfiguration {
                embedding_model_arn:
                    "arn:aws:bedrock:us-west-2::foundation-model/amazon.titan-embed-text-v2:0"
                        .to_owned(),
            },
        },
        storage_configuration: StorageConfiguration {
            storage_type: StorageType::OpensearchServerless,
            aoss: AossConfiguration {
                collection_arn: "arn:aws:aoss:us-west-2:123456789012:collection/collection"
                    .to_owned(),
                field_mapping: AossFieldMapping {
                    metadata_field: "AMAZON_BEDROCK_METADATA".to_owned(),
                    text_field: "AMAZON_BEDROCK_TEXT_CHUNK".to_owned(),
                    vector_field: "bedrock-knowledge-base-default-vector".to_owned(),
                },
                vector_index_name: "bedrock-kb-index".to_owned(),
            },
        },
    }
}

fn data_source_request() -> CreateDataSourceRequest {
    CreateDataSourceRequest {
        client_token: "token".to_owned(),
        name: "kb-s3-source".to_owned(),
        data_source_configuration: NewDataSourceConfiguration {
            data_source_type: DataSourceType::S3,
            s3_configuration: NewS3DataSourceConfiguration {
                bucket_arn: "arn:aws:s3:::bucket".to_owned(),
                inclusion_prefixes: vec!["data/".to_owned()],
            },
        },
    }
}

#[tokio::test]
async fn client_signed_request_resolves_every_credential_key() {
    let transport = FakeTransport::new(vec![json_response("{}")]);
    let resolver = StaticResolver::new(None);
    let parsed = settings(
        r#"{"vector_store_id": "KB123", "aws_access_key_id": "AKID",
            "aws_secret_access_key": "SECRET", "aws_session_token": "TOKEN",
            "aws_session_name": "session", "aws_profile_name": "profile",
            "aws_role_name": "arn:aws:iam::123456789012:role/Role",
            "aws_web_identity_token": "identity", "aws_sts_endpoint": "https://sts.example.com",
            "aws_external_id": "external"}"#,
    );
    clients(parsed, &resolver, transport)
        .signed_request("s3", "GET", "https://example-bucket.s3.amazonaws.com/", &[])
        .await
        .expect("the signed request succeeds");
    let calls = resolver.calls();
    assert_eq!(calls.len(), 1);
    let (sent, region) = &calls[0];
    assert_eq!(region, "us-west-2");
    assert_eq!(secret(sent.access_key_id.as_ref()), Some("AKID"));
    assert_eq!(secret(sent.secret_access_key.as_ref()), Some("SECRET"));
    assert_eq!(secret(sent.session_token.as_ref()), Some("TOKEN"));
    assert_eq!(sent.session_name.as_deref(), Some("session"));
    assert_eq!(sent.profile_name.as_deref(), Some("profile"));
    assert_eq!(
        sent.role_name.as_deref(),
        Some("arn:aws:iam::123456789012:role/Role")
    );
    assert_eq!(secret(sent.web_identity_token.as_ref()), Some("identity"));
    assert_eq!(
        sent.sts_endpoint.as_deref(),
        Some("https://sts.example.com")
    );
    assert_eq!(sent.external_id.as_deref(), Some("external"));
    assert_eq!(sent.region_name.as_deref(), Some("us-west-2"));
    assert!(sent.session_tags.is_none());
    assert!(sent.role_arn.is_none());
    assert!(sent.bearer_token.is_none());
}

#[tokio::test]
async fn client_signed_request_signs_the_resolved_region_and_service() {
    let (built, transport, _resolver) = rag_clients(vec![json_response("{}")]);
    built
        .signed_request(
            "s3",
            "PUT",
            "https://example-bucket.s3.us-west-2.amazonaws.com/",
            b"payload",
        )
        .await
        .expect("the signed request succeeds");
    let requests = transport.requests();
    let request = &requests[0];
    assert_eq!(request.method, "PUT");
    assert_eq!(request.body, b"payload");
    let authorization = header(request, "Authorization").expect("the request is signed");
    assert!(
        authorization.contains("Credential=AKIDRAGEXAMPLE/20260101/us-west-2/s3/aws4_request"),
        "{authorization}"
    );
    assert!(
        authorization.contains("SignedHeaders=host;x-amz-date"),
        "{authorization}"
    );
    assert_eq!(header(request, "Content-Length"), Some("7"));
    assert_eq!(header(request, "X-Amz-Date"), Some("20260101T000000Z"));
}

#[tokio::test]
async fn client_signed_request_with_signs_extra_headers_and_the_signing_region() {
    let (built, transport, resolver) = rag_clients(vec![json_response("{}")]);
    let headers = vec![
        ("Content-Type".to_owned(), "application/json".to_owned()),
        (
            "X-Amz-Target".to_owned(),
            "OpenSearchServerless.CreateCollection".to_owned(),
        ),
    ];
    built
        .signed_request_with(SignedRequest {
            service: "aoss",
            signing_region: "us-east-1",
            method: "POST",
            url: "https://aoss.us-west-2.amazonaws.com/",
            headers: &headers,
            body: b"{}",
        })
        .await
        .expect("the signed request succeeds");
    let requests = transport.requests();
    let authorization = header(&requests[0], "Authorization").expect("the request is signed");
    assert!(
        authorization.contains("Credential=AKIDRAGEXAMPLE/20260101/us-east-1/aoss/aws4_request"),
        "{authorization}"
    );
    assert!(
        authorization.contains("content-type") && authorization.contains("x-amz-target"),
        "{authorization}"
    );
    assert_eq!(resolver.calls()[0].1, "us-west-2");
}

#[tokio::test]
async fn client_signed_request_matches_signed_request_with_no_headers() {
    let (built, transport, _resolver) = rag_clients(vec![json_response("{}"), json_response("{}")]);
    built
        .signed_request("s3", "GET", "https://example-bucket.s3.amazonaws.com/", &[])
        .await
        .expect("the first request succeeds");
    built
        .signed_request_with(SignedRequest {
            service: "s3",
            signing_region: "us-west-2",
            method: "GET",
            url: "https://example-bucket.s3.amazonaws.com/",
            headers: &[],
            body: &[],
        })
        .await
        .expect("the second request succeeds");
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
}

#[tokio::test]
async fn client_session_token_is_sent_and_signed() {
    let transport = FakeTransport::new(vec![json_response("{}")]);
    let resolver = StaticResolver::new(Some("SESSION-TOKEN"));
    let parsed = settings(r#"{"vector_store_id": "KB123", "aws_region_name": "us-west-2"}"#);
    clients(parsed, &resolver, transport.clone())
        .signed_request(
            "bedrock",
            "GET",
            "https://bedrock-agent.us-west-2.amazonaws.com/",
            &[],
        )
        .await
        .expect("the signed request succeeds");
    let requests = transport.requests();
    assert_eq!(
        header(&requests[0], "X-Amz-Security-Token"),
        Some("SESSION-TOKEN")
    );
    let authorization = header(&requests[0], "Authorization").expect("the request is signed");
    assert!(
        authorization.contains("x-amz-security-token"),
        "{authorization}"
    );
}

#[tokio::test]
async fn client_non_2xx_status_becomes_the_aws_error_without_credentials() {
    let error_body = "{\"message\":\"AccessDeniedException\"}";
    let (built, _transport, _resolver) = rag_clients(vec![status_response(403, error_body)]);
    let error = built
        .signed_request("s3", "GET", "https://example-bucket.s3.amazonaws.com/", &[])
        .await
        .and_then(|response| response.require_success::<FakeTransportError>("CreateBucket"))
        .expect_err("a 403 is an error");
    let BedrockIngestionError::Aws {
        operation,
        status,
        body,
    } = &error
    else {
        panic!("expected the AWS error, got {error:?}");
    };
    assert_eq!(*operation, "CreateBucket");
    assert_eq!(*status, 403);
    assert_eq!(body, error_body);
    let rendered = format!("{error}");
    assert!(!rendered.contains(TEST_KEY));
    assert!(!rendered.contains("rag-example-secret"));
    assert!(!rendered.contains("Authorization"));
}

#[tokio::test]
async fn client_error_body_is_capped_at_1024_bytes() {
    let long_body = "x".repeat(2048);
    let (built, _transport, _resolver) = rag_clients(vec![status_response(500, &long_body)]);
    let error = built
        .signed_request("s3", "GET", "https://example-bucket.s3.amazonaws.com/", &[])
        .await
        .and_then(|response| response.require_success::<FakeTransportError>("CreateBucket"))
        .expect_err("a 500 is an error");
    let BedrockIngestionError::Aws { body, .. } = &error else {
        panic!("expected the AWS error, got {error:?}");
    };
    assert_eq!(body.len(), 1024);
}

#[tokio::test]
async fn client_transport_failure_keeps_its_source() {
    let (built, _transport, _resolver) = rag_clients(Vec::new());
    let error = built
        .signed_request("s3", "GET", "https://example-bucket.s3.amazonaws.com/", &[])
        .await
        .expect_err("the fake transport has no answer");
    assert!(
        matches!(error, BedrockIngestionError::Transport { .. }),
        "{error:?}"
    );
}

#[tokio::test]
async fn client_list_data_sources_uses_post_with_an_empty_body() {
    let (built, transport, _resolver) =
        rag_clients(vec![json_response("{\"dataSourceSummaries\":[]}")]);
    let listed = built
        .list_data_sources("KB123")
        .await
        .expect("the list call succeeds");
    assert!(listed.data_source_summaries.is_empty());
    let requests = transport.requests();
    assert_eq!(requests[0].method, "POST");
    assert_eq!(
        requests[0].url,
        format!("{AGENT}/knowledgebases/KB123/datasources/")
    );
    assert!(requests[0].body.is_empty());
    assert_eq!(header(&requests[0], "Content-Type"), None);
    assert_eq!(header(&requests[0], "Content-Length"), Some("0"));
    let authorization = header(&requests[0], "Authorization").expect("the request is signed");
    assert!(
        authorization.contains("Credential=AKIDRAGEXAMPLE/20260101/us-west-2/bedrock/aws4_request"),
        "{authorization}"
    );
}

#[tokio::test]
async fn client_agent_operations_sign_the_partition_region() {
    let transport = FakeTransport::new(vec![json_response(
        "{\"knowledgeBase\":{\"status\":\"ACTIVE\"}}",
    )]);
    let resolver = StaticResolver::new(None);
    let parsed = settings(r#"{"vector_store_id": "KB123", "aws_region_name": "cn-north-1"}"#);
    let built = clients(parsed, &resolver, transport.clone());
    built
        .get_knowledge_base("KB123")
        .await
        .expect("the get call succeeds");
    let requests = transport.requests();
    assert_eq!(
        requests[0].url,
        "https://bedrock-agent.cn-north-1.amazonaws.com.cn/knowledgebases/KB123"
    );
    let authorization = header(&requests[0], "Authorization").expect("the request is signed");
    assert!(
        authorization.contains("/cn-north-1/bedrock/aws4_request"),
        "{authorization}"
    );
}

#[tokio::test]
async fn client_get_data_source_uses_get_without_a_body() {
    let (built, transport, _resolver) = rag_clients(vec![json_response(
        "{\"dataSource\":{\"dataSourceId\":\"DS1\"}}",
    )]);
    let details = built
        .get_data_source("KB123", "DS1")
        .await
        .expect("the get call succeeds");
    assert!(
        details
            .data_source
            .data_source_configuration
            .and_then(|configuration| configuration.s3_configuration)
            .is_none()
    );
    let requests = transport.requests();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].url,
        format!("{AGENT}/knowledgebases/KB123/datasources/DS1")
    );
    assert!(requests[0].body.is_empty());
    assert_eq!(header(&requests[0], "Content-Length"), None);
    assert_eq!(header(&requests[0], "Content-Type"), None);
}

#[tokio::test]
async fn client_create_knowledge_base_sends_the_vector_body() {
    let (built, transport, _resolver) = rag_clients(vec![json_response(
        "{\"knowledgeBase\":{\"knowledgeBaseId\":\"KB-NEW\"}}",
    )]);
    let created = built
        .create_knowledge_base(&knowledge_base_request())
        .await
        .expect("the create knowledge base call succeeds");
    assert_eq!(created.knowledge_base.knowledge_base_id, "KB-NEW");
    let requests = transport.requests();
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(requests[0].url, format!("{AGENT}/knowledgebases/"));
    let body: CreateKnowledgeBaseRequest =
        serde_json::from_slice(&requests[0].body).expect("the request body");
    assert_eq!(body, knowledge_base_request());
    assert_eq!(
        header(&requests[0], "Content-Type"),
        Some("application/json")
    );
}

#[tokio::test]
async fn client_get_knowledge_base_reads_the_status() {
    let (built, transport, _resolver) = rag_clients(vec![json_response(
        "{\"knowledgeBase\":{\"knowledgeBaseId\":\"KB-NEW\",\"status\":\"ACTIVE\"}}",
    )]);
    let state = built
        .get_knowledge_base("KB-NEW")
        .await
        .expect("the get knowledge base call succeeds");
    assert_eq!(state.knowledge_base.status, "ACTIVE");
    let requests = transport.requests();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].url, format!("{AGENT}/knowledgebases/KB-NEW"));
}

#[tokio::test]
async fn client_create_data_source_sends_the_s3_body() {
    let (built, transport, _resolver) = rag_clients(vec![json_response(
        "{\"dataSource\":{\"dataSourceId\":\"DS-NEW\"}}",
    )]);
    let created = built
        .create_data_source("KB-NEW", &data_source_request())
        .await
        .expect("the create data source call succeeds");
    assert_eq!(created.data_source.data_source_id, "DS-NEW");
    let requests = transport.requests();
    assert_eq!(requests[0].method, "PUT");
    assert_eq!(
        requests[0].url,
        format!("{AGENT}/knowledgebases/KB-NEW/datasources/")
    );
    let body: CreateDataSourceRequest =
        serde_json::from_slice(&requests[0].body).expect("the request body");
    assert_eq!(body, data_source_request());
}

#[tokio::test]
async fn client_operations_encode_the_path_ids() {
    let (built, transport, _resolver) =
        rag_clients(vec![json_response("{\"dataSourceSummaries\":[]}")]);
    built
        .list_data_sources("KB 1/2")
        .await
        .expect("the list call succeeds");
    let requests = transport.requests();
    assert_eq!(
        requests[0].url,
        format!("{AGENT}/knowledgebases/KB%201%2F2/datasources/")
    );
}

#[tokio::test]
async fn client_operations_name_the_failing_operation() {
    let (built, _transport, _resolver) = rag_clients(vec![status_response(500, "boom")]);
    let error = built
        .list_data_sources("KB123")
        .await
        .expect_err("a 500 is an error");
    let BedrockIngestionError::Aws {
        operation, status, ..
    } = &error
    else {
        panic!("expected the AWS error, got {error:?}");
    };
    assert_eq!(*operation, "ListDataSources");
    assert_eq!(*status, 500);
}

#[tokio::test]
async fn client_malformed_response_is_a_typed_error() {
    let (built, _transport, _resolver) = rag_clients(vec![json_response("not json")]);
    let error = built
        .list_data_sources("KB123")
        .await
        .expect_err("the body is not JSON");
    assert!(
        matches!(
            error,
            BedrockIngestionError::Response {
                operation: "ListDataSources",
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn client_agent_endpoints_follow_the_partition() {
    assert_eq!(
        bedrock_agent_endpoint("us-west-2"),
        "https://bedrock-agent.us-west-2.amazonaws.com"
    );
    assert_eq!(
        bedrock_agent_endpoint("cn-north-1"),
        "https://bedrock-agent.cn-north-1.amazonaws.com.cn"
    );
    assert_eq!(
        bedrock_agent_endpoint("us-gov-west-1"),
        "https://bedrock-agent.us-gov-west-1.amazonaws.com"
    );
}

#[test]
fn client_s3_bucket_hosts_follow_boto3() {
    assert_eq!(
        s3_bucket_host("litellm-kb-abcd1234", "us-east-1"),
        "https://litellm-kb-abcd1234.s3.amazonaws.com"
    );
    assert_eq!(
        s3_bucket_host("litellm-kb-abcd1234", "us-west-2"),
        "https://litellm-kb-abcd1234.s3.us-west-2.amazonaws.com"
    );
    assert_eq!(
        s3_bucket_host("litellm-kb-abcd1234", "cn-north-1"),
        "https://litellm-kb-abcd1234.s3.cn-north-1.amazonaws.com.cn"
    );
    assert_eq!(
        s3_bucket_host("my.dotted.bucket", "us-east-1"),
        "https://s3.us-east-1.amazonaws.com/my.dotted.bucket"
    );
    assert_eq!(
        s3_bucket_host("UpperBucket", "us-gov-west-1"),
        "https://s3.us-gov-west-1.amazonaws.com/UpperBucket"
    );
}

#[test]
fn client_create_bucket_body_only_exists_outside_us_east_1() {
    assert!(create_bucket_body("us-east-1").is_none());
    assert_eq!(
        create_bucket_body("eu-west-1").as_deref(),
        Some(
            "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><LocationConstraint>eu-west-1</LocationConstraint></CreateBucketConfiguration>"
        )
    );
}

#[test]
fn client_aws_settings_carry_the_resolved_region() {
    let parsed = settings(r#"{"vector_store_id": "KB123", "aws_region_name": "cn-north-1"}"#);
    let built = aws_settings(&parsed);
    assert_eq!(built.region_name.as_deref(), Some("cn-north-1"));
}

#[test]
fn client_system_clock_tracks_the_wall_clock() {
    let wall = SystemTime::now();
    let reported = SystemClock.now();
    let drift = match reported.duration_since(wall) {
        Ok(ahead) => ahead,
        Err(_behind) => wall.duration_since(reported).unwrap_or_default(),
    };
    assert!(drift < Duration::from_secs(1));
}

struct Ports;

impl OidcHttp for Ports {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the rag fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for Ports {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for Ports {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

#[derive(Debug)]
struct NoNetwork;

impl HttpConnector for NoNetwork {
    fn call(&self, _request: HttpRequest) -> HttpConnectorFuture {
        HttpConnectorFuture::ready(Err(ConnectorError::other(
            Box::new(io::Error::other("the rag fixture has no network")),
            None,
        )))
    }
}

fn rag_cache(events: Arc<Mutex<Vec<String>>>) -> CredentialCache {
    let memory = LocalTtlCache::with_clock(
        Some(1000),
        None,
        None,
        1024,
        Arc::new(|| f64::from(u32::try_from(NOW_SECONDS).unwrap())),
    )
    .unwrap();
    CredentialCache::new(DualLocalCache {
        memory: Arc::new(Mutex::new(memory)),
        default_in_memory_ttl: None,
        diagnostics: Diagnostics {
            redactor: Arc::new(Redactor::new(16).unwrap()),
            emit: Arc::new(move |event: CacheEvent| {
                events
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(event.message);
            }),
        },
    })
    .unwrap()
}

#[tokio::test]
async fn client_b594_resolver_forwards_static_keys() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let cache = rag_cache(Arc::clone(&events));
    let native = NativeSources {
        environment: BTreeMap::new(),
        filesystem: Fs::from_slice(&[]),
        client: http_client_fn(|_settings, _runtime| SharedHttpConnector::new(NoNetwork)),
        clock: SharedTimeSource::new(StaticTimeSource::new(fixed_now())),
        sleep: SharedAsyncSleep::new(TokioSleep::new()),
        timeout: Duration::from_secs(5),
    };
    let secrets = SecretLookup {
        manager: None,
        oidc_http: Arc::new(Ports),
        azure_tokens: Arc::new(Ports),
        clock: Arc::new(Ports),
        oidc_cache: OidcTokenCache::default(),
    };
    let sts = StsClientKwargs {
        verify: Verification::Enabled,
        endpoint_url: None,
        region_name: None,
    };
    let resolver = AwsCredentialResolver {
        cache: &cache,
        native: &native,
        secrets: &secrets,
        sts: &sts,
        max_policy_size: 75,
    };
    let parsed = settings(
        r#"{"vector_store_id": "KB123", "aws_access_key_id": "AKID", "aws_secret_access_key": "SECRET"}"#,
    );
    let credentials = resolver
        .credentials(&aws_settings(&parsed), "us-west-2")
        .await
        .expect("the static keys resolve without a network");
    assert_eq!(credentials.access_key_id.expose_secret(), "AKID");
    assert_eq!(credentials.secret_access_key.expose_secret(), "SECRET");
    assert!(credentials.session_token.is_none());
}
