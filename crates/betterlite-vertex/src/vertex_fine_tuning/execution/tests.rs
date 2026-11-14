#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, Ready, ready};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_openai::fine_tuning::job::{FineTuneCreate, JobStatus};
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap};
use jiff::tz::TimeZone;
use secrecy::SecretString;

use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_fine_tuning::execution::TuningTransport;
use crate::vertex_fine_tuning::execution::create::{
    TUNING_TIMEOUT, TuningCreateRequest, create_tuning_job,
};
use crate::vertex_fine_tuning::records::request::{
    OriginalHyperparameters, SupervisedTuningSpec, VertexHyperparameters, VertexTuningJobCreate,
};

const TUNING_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/tuningJobs";

const JOB: &str = r#"{"name":"projects/proj/locations/us-central1/tuningJobs/7","baseModel":"gemini-1.5-flash-002","tunedModelDisplayName":"tuned","supervisedTuningSpec":{"trainingDatasetUri":"gs://bucket/train.jsonl"},"state":"JOB_STATE_SUCCEEDED","createTime":"2024-01-01T00:00:00Z"}"#;

type Order = Arc<Mutex<Vec<&'static str>>>;

#[derive(Clone)]
enum Reply {
    Status { status: u16, body: Vec<u8> },
    Failure,
}

struct Recorded {
    method: Method,
    url: String,
    headers: HeaderMap,
    deadline: Duration,
    body: Vec<u8>,
}

struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Recorded>>,
    order: Order,
}

impl TuningTransport for Scripted {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.order.lock().unwrap().push("post");
        let body = match request.body {
            RequestBody::Json(data) => data,
            RequestBody::Bytes { data, .. } => data,
            RequestBody::Empty | RequestBody::Multipart { .. } => Vec::new(),
        };
        self.requests.lock().unwrap().push(Recorded {
            method: request.method,
            url: request.url.to_string(),
            headers: request.headers,
            deadline: request.deadline,
            body,
        });
        let reply = self.replies.lock().unwrap().pop_front().unwrap();
        ready(match reply {
            Reply::Status { status, body } => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Reply::Failure => Err(TransportError::Closed),
        })
    }
}

struct Fixture {
    catalog: Arc<ModelCatalog>,
    data: FineTuneCreate,
    original: OriginalHyperparameters,
    local: LocalTimeContext,
}

impl Fixture {
    fn new() -> Self {
        Self {
            catalog: BundledCatalog::default().get(),
            data: serde_json::from_str(
                r#"{"model":"gemini-1.5-flash-002","training_file":"gs://bucket/train.jsonl","hyperparameters":{"n_epochs":2}}"#,
            )
            .unwrap(),
            original: OriginalHyperparameters { adapter_size: None },
            local: LocalTimeContext {
                zone: TimeZone::get("UTC").unwrap(),
            },
        }
    }

    fn request<'a>(
        &'a self,
        api_base: Option<&'a str>,
        timeout: Option<Duration>,
    ) -> TuningCreateRequest<'a> {
        TuningCreateRequest {
            catalog: &self.catalog,
            data: &self.data,
            original_hyperparameters: &self.original,
            credentials: None,
            project: Some("proj"),
            location: Some("us-central1"),
            api_base,
            local: &self.local,
            timeout,
        }
    }
}

fn access() -> AccessToken {
    AccessToken {
        token: SecretString::from("token-abc"),
        project: "proj".to_owned(),
    }
}

fn lookup(
    _credentials: Option<&ConfiguredCredentials>,
    _project: Option<&str>,
) -> Ready<Result<AccessToken, CredentialCacheError>> {
    ready(Ok::<AccessToken, CredentialCacheError>(access()))
}

fn reply(status: u16, body: &str) -> Reply {
    Reply::Status {
        status,
        body: body.as_bytes().to_vec(),
    }
}

fn scripted(replies: Vec<Reply>, order: &Order) -> Scripted {
    Scripted {
        replies: Mutex::new(VecDeque::from(replies)),
        requests: Mutex::new(Vec::new()),
        order: Arc::clone(order),
    }
}

#[tokio::test]
async fn token_before_post() {
    let fixture = Fixture::new();
    let order: Order = Arc::new(Mutex::new(Vec::new()));
    let transport = scripted(vec![reply(200, JOB)], &order);
    let recorded_order = Arc::clone(&order);
    let make_lookup = move |_credentials: Option<&ConfiguredCredentials>,
                            _project: Option<&str>| {
        recorded_order.lock().unwrap().push("token");
        ready(Ok::<AccessToken, CredentialCacheError>(access()))
    };
    let job = create_tuning_job(&transport, &fixture.request(None, None), make_lookup)
        .await
        .unwrap();
    assert_eq!(*order.lock().unwrap(), ["token", "post"]);
    assert_eq!(job.id, "projects/proj/locations/us-central1/tuningJobs/7");
    assert_eq!(job.status, JobStatus::Succeeded);
    assert_eq!(job.created_at, 1_704_067_200);
    assert_eq!(job.training_file, "gs://bucket/train.jsonl");
    let recorded = &transport.requests.lock().unwrap()[0];
    assert_eq!(recorded.method, Method::POST);
    assert_eq!(recorded.url, TUNING_URL);
    assert_eq!(
        recorded
            .headers
            .get(AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer token-abc"
    );
    assert_eq!(
        recorded
            .headers
            .get(CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap(),
        "application/json"
    );
    assert_eq!(
        serde_json::from_slice::<VertexTuningJobCreate>(&recorded.body).unwrap(),
        VertexTuningJobCreate {
            base_model: "gemini-1.5-flash-002".to_owned(),
            supervised_tuning_spec: SupervisedTuningSpec {
                training_dataset_uri: "gs://bucket/train.jsonl".to_owned(),
                validation_dataset: None,
                hyperparameters: Some(VertexHyperparameters {
                    epoch_count: Some(2),
                    learning_rate_multiplier: None,
                    adapter_size: None,
                }),
            },
            tuned_model_display_name: None,
        }
    );
}

#[tokio::test]
async fn fixed_timeout() {
    let fixture = Fixture::new();
    let order: Order = Arc::new(Mutex::new(Vec::new()));
    let transport = scripted(vec![reply(200, JOB), reply(200, JOB)], &order);
    create_tuning_job(
        &transport,
        &fixture.request(None, Some(Duration::from_secs(5))),
        lookup,
    )
    .await
    .unwrap();
    create_tuning_job(&transport, &fixture.request(None, None), lookup)
        .await
        .unwrap();
    let deadlines: Vec<Duration> = transport
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|recorded| recorded.deadline)
        .collect();
    assert_eq!(deadlines, [TUNING_TIMEOUT, TUNING_TIMEOUT]);
    assert_eq!(TUNING_TIMEOUT, Duration::from_secs(600));
}

#[tokio::test]
async fn ignores_custom_target() {
    let fixture = Fixture::new();
    let order: Order = Arc::new(Mutex::new(Vec::new()));
    let transport = scripted(vec![reply(200, JOB), reply(200, JOB)], &order);
    create_tuning_job(&transport, &fixture.request(None, None), lookup)
        .await
        .unwrap();
    create_tuning_job(
        &transport,
        &fixture.request(Some("https://proxy.example.com/v1beta1"), None),
        lookup,
    )
    .await
    .unwrap();
    let urls: Vec<String> = transport
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|recorded| recorded.url.clone())
        .collect();
    assert_eq!(urls, [TUNING_URL, TUNING_URL]);
    let error = create_tuning_job(
        &transport,
        &fixture.request(Some("not a url"), None),
        lookup,
    )
    .await
    .unwrap_err();
    assert_eq!(error.status_code(), 500);
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn error_status_body() {
    let fixture = Fixture::new();
    let order: Order = Arc::new(Mutex::new(Vec::new()));
    let transport = scripted(vec![reply(400, "invalid tuning job")], &order);
    let error = create_tuning_job(&transport, &fixture.request(None, None), lookup)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), 500);
    assert!(error.to_string().contains("invalid tuning job"), "{error}");
    assert!(error.to_string().contains("400"), "{error}");
}

#[tokio::test]
async fn only_a_200_is_accepted() {
    let fixture = Fixture::new();
    let order: Order = Arc::new(Mutex::new(Vec::new()));
    let transport = scripted(vec![reply(201, JOB)], &order);
    let error = create_tuning_job(&transport, &fixture.request(None, None), lookup)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), 500);
    assert!(error.to_string().contains("201"), "{error}");
}

#[tokio::test]
async fn a_transport_failure_is_a_500() {
    let fixture = Fixture::new();
    let order: Order = Arc::new(Mutex::new(Vec::new()));
    let transport = scripted(vec![Reply::Failure], &order);
    let error = create_tuning_job(&transport, &fixture.request(None, None), lookup)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn a_missing_location_fails_before_the_post() {
    let fixture = Fixture::new();
    let order: Order = Arc::new(Mutex::new(Vec::new()));
    let transport = scripted(vec![reply(200, JOB)], &order);
    let mut request = fixture.request(None, None);
    request.location = None;
    let error = create_tuning_job(&transport, &request, lookup)
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), 500);
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancel_create() {
    let fixture = Fixture::new();
    let order: Order = Arc::new(Mutex::new(Vec::new()));
    let transport = scripted(vec![reply(200, JOB)], &order);
    let request = fixture.request(None, None);
    let recorded_order = Arc::clone(&order);
    let make_lookup = move |_credentials: Option<&ConfiguredCredentials>,
                            _project: Option<&str>| {
        recorded_order.lock().unwrap().push("token");
        ready(Ok::<AccessToken, CredentialCacheError>(access()))
    };
    let pending = create_tuning_job(&transport, &request, make_lookup);
    drop(pending);
    assert!(order.lock().unwrap().is_empty());
    assert!(transport.requests.lock().unwrap().is_empty());
}
