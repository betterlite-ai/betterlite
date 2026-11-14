#![cfg(test)]

#[path = "tests/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::error::Error as _;
use std::future::ready;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::str::from_utf8;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};
use betterlite_openai::api::batch_records::CreateBatchRequest;
use betterlite_openai::batches::records::BatchObject;
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use jiff::tz::TimeZone;
use rstest::rstest;
use tokio::sync::oneshot;
use tokio::time::timeout;
use uuid::Uuid;

use super::{
    BatchCreateRequest, DEFAULT_CLIENT_DEADLINE, execute_batch_create, prepare_batch_create,
};
use crate::cache_lookup::transport::CacheTransport;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::vertex_batch_create::error::VertexBatchCreateError;
use crate::vertex_batch_create::fetch::{ProtectedGet, SafeGet};
use crate::vertex_batch_records::types::VertexAIBatchPredictionJob;
use support::{JOB, Reply, Scripted, access, reply, reply_bytes, stalling_server};

const TUNED: &str = "gs://bucket/endpoints/222/file.jsonl";
const PUBLISHER: &str = "gs://bucket/publishers/google/models/gemini-pro/file.jsonl";
const HOST: &str =
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1";
const DEPLOYED: &str = r#"{"deployedModels":[{"model":"projects/proj/locations/us-central1/models/9"},{"model":"projects/proj/locations/us-central1/models/10"}]}"#;

fn create(input: &str) -> CreateBatchRequest {
    CreateBatchRequest {
        completion_window: None,
        endpoint: None,
        input_file_id: Some(input.to_owned()),
        metadata: None,
        output_expires_after: None,
        extra_headers: None,
        extra_body: None,
        timeout: None,
    }
}

fn base(create: &CreateBatchRequest) -> BatchCreateRequest<'_> {
    BatchCreateRequest {
        create,
        api_base: None,
        project: Some("proj"),
        location: Some("us-central1"),
        custom_endpoint: false,
        display_id: Uuid::nil(),
        deadline: DEFAULT_CLIENT_DEADLINE,
        timeout: None,
        max_retries: None,
    }
}

async fn run<Transport, Fetch>(
    transport: &Transport,
    fetcher: &Fetch,
    request: &BatchCreateRequest<'_>,
) -> Result<BatchObject, VertexBatchCreateError>
where
    Transport: CacheTransport,
    Fetch: ProtectedGet,
{
    let prepared = prepare_batch_create(request, ready(access("tokproj"))).await?;
    let local = LocalTimeContext {
        zone: TimeZone::get("UTC").unwrap(),
    };
    execute_batch_create(transport, fetcher, prepared, &local).await
}

fn job_body(body: &[u8]) -> VertexAIBatchPredictionJob {
    serde_json::from_slice(body).unwrap()
}

struct BlockedResolver;

#[async_trait]
impl Resolve for BlockedResolver {
    async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        Ok(vec![SocketAddr::from((
            Ipv4Addr::new(169, 254, 169, 254),
            443,
        ))])
    }
}

#[tokio::test]
async fn custom_endpoint_before_auth() {
    let polled = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&polled);
    let input = create(TUNED);
    let request = BatchCreateRequest {
        custom_endpoint: true,
        ..base(&input)
    };
    let error = prepare_batch_create(&request, async move {
        seen.store(true, Ordering::SeqCst);
        access("proj")
    })
    .await
    .unwrap_err();
    assert!(matches!(error, VertexBatchCreateError::CustomEndpoint));
    assert_eq!(error.status_code(), 400);
    assert!(error.to_string().contains("`custom_endpoint` deployments"));
    assert!(!polled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn endpoint_lookup_before_create() {
    let transport = Scripted::new(vec![reply(200, DEPLOYED), reply(200, JOB)]);
    let object = run(&transport, &transport, &base(&create(TUNED)))
        .await
        .unwrap();
    let seen = transport.requests();
    let calls: Vec<(Method, String)> = seen
        .iter()
        .map(|call| (call.method.clone(), call.url.clone()))
        .collect();
    assert_eq!(
        calls,
        vec![
            (Method::GET, format!("{HOST}/endpoints/222")),
            (Method::POST, format!("{HOST}/batchPredictionJobs")),
        ]
    );
    assert!(seen[0].body.is_empty());
    assert_eq!(object.id, "777");
    let headers: Vec<(&str, bool, &str)> = seen
        .iter()
        .map(|call| {
            let token = call.headers.get(AUTHORIZATION).unwrap();
            (
                token.to_str().unwrap(),
                token.is_sensitive(),
                call.headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
            )
        })
        .collect();
    let expected = ("Bearer tok-123", true, "application/json; charset=utf-8");
    assert_eq!(headers, vec![expected, expected]);
}

#[tokio::test]
async fn endpoint_lookup_is_ssrf_checked() {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a proxy policy"),
    );
    let client =
        ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).expect("a client");
    let allowed: Vec<String> = Vec::new();
    let validator = UrlValidator::new(BlockedResolver, &allowed, &Verification::Enabled);
    let urls = UserUrlFetcher {
        validator,
        validation: true,
    };
    let safe = SafeGet {
        urls: &urls,
        client: &client,
    };
    let transport = Scripted::new(Vec::new());
    let error = run(&transport, &safe, &base(&create(TUNED)))
        .await
        .unwrap_err();
    assert!(matches!(error, VertexBatchCreateError::Lookup { .. }));
    assert_eq!(error.status_code(), 500);
    assert!(error.to_string().contains("blocked address"));
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn deployed_model_replacement() {
    let transport = Scripted::new(vec![reply(200, DEPLOYED), reply(200, JOB)]);
    run(&transport, &transport, &base(&create(TUNED)))
        .await
        .unwrap();
    let job = job_body(&transport.requests()[1].body);
    assert_eq!(job.model, "projects/proj/locations/us-central1/models/9");
    assert_eq!(
        job.display_name,
        "litellm-vertex-batch-00000000-0000-0000-0000-000000000000"
    );
    assert_eq!(job.input_config.gcs_source.uris[0], TUNED);
}

#[rstest]
#[case::empty_first_with_later_model(
    r#"{"deployedModels":[{"model":""},{"model":"projects/p/locations/l/models/2"}]}"#
)]
#[case::missing_first_model(
    r#"{"deployedModels":[{},{"model":"projects/p/locations/l/models/2"}]}"#
)]
#[case::null_first_model(
    r#"{"deployedModels":[{"model":null},{"model":"projects/p/locations/l/models/2"}]}"#
)]
#[case::empty_list(r#"{"deployedModels":[]}"#)]
#[case::null_list(r#"{"deployedModels":null}"#)]
#[case::missing_list(r"{}")]
#[tokio::test]
async fn first_deployed_model_only(#[case] endpoint: &str) {
    let transport = Scripted::new(vec![reply(200, endpoint)]);
    let error = run(&transport, &transport, &base(&create(TUNED)))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VertexBatchCreateError::NoDeployedModel { .. }
    ));
    assert_eq!(error.status_code(), 400);
    assert!(error.to_string().contains("has no deployed model"));
    assert_eq!(transport.requests().len(), 1);
}

#[rstest]
#[case::not_found(404)]
#[case::forbidden(403)]
#[case::server_error(500)]
#[case::created(201)]
#[case::no_content(204)]
#[tokio::test]
async fn lookup_status_propagation(#[case] status: u16) {
    let transport = Scripted::new(vec![reply(status, "no endpoint here")]);
    let error = run(&transport, &transport, &base(&create(TUNED)))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VertexBatchCreateError::EndpointStatus { .. }
    ));
    assert_eq!(error.status_code(), status);
    assert_eq!(
        error.to_string(),
        "Failed to resolve fine-tuned Vertex endpoint 'projects/proj/locations/us-central1/endpoints/222': no endpoint here"
    );
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn async_no_blocking() {
    let (open, gate) = oneshot::channel();
    let transport = Scripted::new(vec![reply(200, JOB)]).gated(gate);
    let fetcher = Scripted::new(Vec::new());
    let input = create(PUBLISHER);
    let request = base(&input);
    let released = async {
        assert!(
            transport.entered.load(Ordering::SeqCst),
            "the create reached the transport and yielded"
        );
        open.send(()).unwrap();
    };
    let (result, ()) = timeout(Duration::from_secs(5), async {
        tokio::join!(run(&transport, &fetcher, &request), released)
    })
    .await
    .unwrap();
    assert_eq!(result.unwrap().id, "777");
}

#[tokio::test]
async fn timeout_not_forwarded() {
    let transport = Scripted::new(vec![reply(200, DEPLOYED), reply(200, JOB)]);
    let mut input = create(TUNED);
    input.timeout = Some(1.5);
    let request = BatchCreateRequest {
        timeout: Some(2.5),
        max_retries: Some(9),
        ..base(&input)
    };
    run(&transport, &transport, &request).await.unwrap();
    let seen = transport.requests();
    let deadlines: Vec<Duration> = seen.iter().map(|call| call.deadline).collect();
    assert_eq!(deadlines, vec![DEFAULT_CLIENT_DEADLINE; 2]);
    let body = from_utf8(&seen[1].body).unwrap();
    assert!(!body.contains("\"timeout\""));
    assert!(!body.contains("\"maxRetries\""));
}

#[tokio::test]
async fn client_deadline_reaches_both_requests() {
    let transport = Scripted::new(vec![reply(200, DEPLOYED), reply(200, JOB)]);
    let input = create(TUNED);
    let request = BatchCreateRequest {
        deadline: Duration::from_secs(42),
        ..base(&input)
    };
    run(&transport, &transport, &request).await.unwrap();
    let deadlines: Vec<Duration> = transport
        .requests()
        .iter()
        .map(|call| call.deadline)
        .collect();
    assert_eq!(deadlines, vec![Duration::from_secs(42); 2]);
}

#[tokio::test]
async fn publisher_skips_resolution() {
    let transport = Scripted::new(vec![reply(200, JOB)]);
    let fetcher = Scripted::new(Vec::new());
    run(&transport, &fetcher, &base(&create(PUBLISHER)))
        .await
        .unwrap();
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::POST);
    assert_eq!(
        job_body(&seen[0].body).model,
        "publishers/google/models/gemini-pro"
    );
}

#[rstest]
#[case::given(
    Some("p1"),
    Some("europe-west4"),
    "https://europe-west4-aiplatform.googleapis.com/v1/projects/p1/locations/europe-west4/batchPredictionJobs"
)]
#[case::absent(
    None,
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/tokproj/locations/us-central1/batchPredictionJobs"
)]
#[case::empty(
    Some(""),
    Some(""),
    "https://us-central1-aiplatform.googleapis.com/v1/projects/tokproj/locations/us-central1/batchPredictionJobs"
)]
#[tokio::test]
async fn project_and_location_defaults(
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] expected: &str,
) {
    let transport = Scripted::new(vec![reply(200, JOB)]);
    let fetcher = Scripted::new(Vec::new());
    let input = create(PUBLISHER);
    let request = BatchCreateRequest {
        project,
        location,
        ..base(&input)
    };
    run(&transport, &fetcher, &request).await.unwrap();
    assert_eq!(transport.requests()[0].url, expected);
}

#[tokio::test]
async fn token_project_names_the_tuned_endpoint() {
    let transport = Scripted::new(vec![reply(200, DEPLOYED), reply(200, JOB)]);
    let input = create(TUNED);
    let request = BatchCreateRequest {
        project: None,
        location: Some("europe-west4"),
        ..base(&input)
    };
    run(&transport, &transport, &request).await.unwrap();
    assert_eq!(
        transport.requests()[0].url,
        "https://europe-west4-aiplatform.googleapis.com/v1/projects/tokproj/locations/europe-west4/endpoints/222"
    );
}

#[rstest]
#[case::host_only(
    "https://proxy.example.com",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/endpoints/222",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs"
)]
#[case::version_root(
    "https://proxy.example.com/v1",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/endpoints/222",
    "https://proxy.example.com/v1/projects/proj/locations/us-central1/batchPredictionJobs"
)]
#[case::mount_prefix(
    "https://proxy.example.com/gw",
    "https://proxy.example.com/gw/v1/projects/proj/locations/us-central1/endpoints/222",
    "https://proxy.example.com/gw://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/batchPredictionJobs"
)]
#[tokio::test]
async fn custom_base_urls(#[case] base_url: &str, #[case] lookup: &str, #[case] post: &str) {
    let transport = Scripted::new(vec![reply(200, DEPLOYED), reply(200, JOB)]);
    let input = create(TUNED);
    let request = BatchCreateRequest {
        api_base: Some(base_url),
        ..base(&input)
    };
    run(&transport, &transport, &request).await.unwrap();
    let urls: Vec<String> = transport.requests().into_iter().map(|c| c.url).collect();
    assert_eq!(urls, vec![lookup.to_owned(), post.to_owned()]);
}

#[rstest]
#[case::bad_request(400, "bad job")]
#[case::quota(429, "slow down")]
#[case::server(503, "try later")]
#[tokio::test]
async fn create_status_is_preserved(#[case] status: u16, #[case] body: &str) {
    let transport = Scripted::new(vec![reply(status, body)]);
    let fetcher = Scripted::new(Vec::new());
    let error = run(&transport, &fetcher, &base(&create(PUBLISHER)))
        .await
        .unwrap_err();
    assert!(matches!(error, VertexBatchCreateError::Http { .. }));
    assert_eq!(error.status_code(), status);
    assert!(error.to_string().contains(body));
}

#[rstest]
#[case::ok(200, true)]
#[case::created(201, true)]
#[case::last_success(299, true)]
#[case::multiple_choices(300, false)]
#[case::not_modified(304, false)]
#[case::permanent_redirect(308, false)]
#[tokio::test]
async fn only_a_2xx_create_is_a_success(#[case] status: u16, #[case] succeeds: bool) {
    let transport = Scripted::new(vec![reply(status, JOB)]);
    let fetcher = Scripted::new(Vec::new());
    let outcome = run(&transport, &fetcher, &base(&create(PUBLISHER))).await;
    if succeeds {
        assert_eq!(outcome.unwrap().id, "777");
    } else {
        let error = outcome.unwrap_err();
        assert!(matches!(error, VertexBatchCreateError::Http { .. }));
        assert_eq!(error.status_code(), status);
    }
}

#[rstest]
#[case::create(PUBLISHER, vec![reply_bytes(400, b"bad \xff job")], "bad ")]
#[case::lookup(TUNED, vec![reply_bytes(404, b"gone \xfe")], "gone ")]
#[tokio::test]
async fn an_error_body_keeps_the_text_before_its_first_invalid_byte(
    #[case] input: &str,
    #[case] replies: Vec<Reply>,
    #[case] kept: &str,
) {
    let transport = Scripted::new(replies);
    let error = run(&transport, &transport, &base(&create(input)))
        .await
        .unwrap_err();
    assert!(error.to_string().ends_with(&format!(": {kept}")));
}

#[tokio::test]
async fn the_lookup_deadline_covers_the_whole_get() {
    let server = stalling_server().await;
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a proxy policy"),
    );
    let client =
        ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).expect("a client");
    let urls = UserUrlFetcher {
        validator: UrlValidator::new(BlockedResolver, &[], &Verification::Enabled),
        validation: false,
    };
    let safe = SafeGet {
        urls: &urls,
        client: &client,
    };
    let transport = Scripted::new(Vec::new());
    let input = create(TUNED);
    let request = BatchCreateRequest {
        api_base: Some(&server.base),
        deadline: Duration::from_millis(200),
        ..base(&input)
    };
    let error = run(&transport, &safe, &request).await.unwrap_err();
    assert!(matches!(error, VertexBatchCreateError::LookupTimeout));
    assert_eq!(error.status_code(), 500);
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn transport_failure_is_a_500() {
    let transport = Scripted::new(vec![Reply::Failure]);
    let fetcher = Scripted::new(Vec::new());
    let error = run(&transport, &fetcher, &base(&create(PUBLISHER)))
        .await
        .unwrap_err();
    assert!(matches!(error, VertexBatchCreateError::Transport { .. }));
    assert!(error.source().is_some());
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn a_stalled_provider_is_a_408() {
    let server = stalling_server().await;
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a proxy policy"),
    );
    let limits = TransportLimits {
        read_timeout: Duration::from_millis(300),
        ..TransportLimits::DEFAULT
    };
    let client = ProxiedClient::build(base_builder(), &policy, limits).expect("a fixture client");
    let fetcher = Scripted::new(Vec::new());
    let input = create(PUBLISHER);
    let request = BatchCreateRequest {
        api_base: Some(&server.base),
        ..base(&input)
    };
    let error = run(&client, &fetcher, &request).await.unwrap_err();
    assert!(matches!(error, VertexBatchCreateError::Timeout { .. }));
    assert_eq!(error.status_code(), 408);
}

#[tokio::test]
async fn missing_input_file_is_not_a_provider_error() {
    let transport = Scripted::new(Vec::new());
    let fetcher = Scripted::new(Vec::new());
    let input = CreateBatchRequest::default();
    let error = run(&transport, &fetcher, &base(&input)).await.unwrap_err();
    assert!(matches!(error, VertexBatchCreateError::Records { .. }));
    assert_eq!(error.status_code(), 500);
    assert!(transport.requests().is_empty());
}
