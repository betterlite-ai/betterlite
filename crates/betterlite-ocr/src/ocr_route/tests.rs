#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber;
use tracing::{Event, Metadata, Subscriber};

use crate::json::{Json, Object};
use crate::ocr_auth::AzureTokenSource;
use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_config::connection::{OcrInputSource, OcrSourced};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrError, OcrSecretError};
use crate::ocr_http::tests::support::{MockServer, Reply, ocr_client, reply};
use crate::ocr_http::{OcrClient, SecretSource};
use crate::ocr_input::wire::{OcrWireRequest, decode_request, decode_request_input};
use crate::ocr_input::{LiteLLMOcrRequest, OcrConnectionInputs, OcrDocumentInput};
use crate::ocr_route::{OcrCall, OcrHost, OcrMachine, OcrOp, OcrRoute, aocr};
use crate::ocr_settings::OcrSettings;
use crate::response::OCRResponse;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

const ANSWER: &[u8] =
    br##"{"pages":[{"index":0,"markdown":"# Title"}],"usage_info":{"pages_processed":1}}"##;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: &[(&str, Json)]) -> Object {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn document(source: &str) -> Json {
    Json::Object(object(&[
        ("type", text("document_url")),
        ("document_url", text(source)),
    ]))
}

fn wire(model: &str, document: Json, api_base: Option<String>) -> OcrWireRequest {
    OcrWireRequest {
        model: model.to_owned(),
        document,
        api_key: Some(SecretString::from("sk-test")),
        api_base,
        custom_llm_provider: None,
        extra_headers: None,
        optional_params: Object::new(),
        input_sources: BTreeMap::new(),
        timeout_seconds: None,
    }
}

fn answer_reply() -> Reply {
    reply("200 OK", &[("content-type", "application/json")], ANSWER)
}

fn client_with(source: Arc<dyn AzureTokenSource>) -> OcrClient {
    let mut client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    client.azure = source;
    client
}

fn captured_body(captured: &str) -> Object {
    let body = captured
        .split_once("\r\n\r\n")
        .expect("the request has a body")
        .1;
    let decoded: Json = serde_json::from_str(body).expect("the body is JSON");
    let Json::Object(fields) = decoded else {
        panic!("the body is an object");
    };
    fields
}

fn assert_mistral_response(response: &OCRResponse) {
    assert_eq!(response.model, "mistral-ocr-latest");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(
        response.pages.first().expect("one page").markdown,
        "# Title"
    );
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.pages_processed),
        Some(1)
    );
}

struct CountingAzure {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl AzureTokenSource for CountingAzure {
    async fn token(
        &self,
        request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        let _count = self.calls.fetch_add(1, Ordering::SeqCst);
        match &request.azure_ad_token_provider {
            Some(provider) => {
                let value = provider.token().await?;
                Ok(Some(OcrSourced {
                    value,
                    source: OcrInputSource::Request,
                }))
            }
            None => Ok(None),
        }
    }
}

struct CountingHost {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl OcrHost for CountingHost {
    async fn call(&self, op: OcrOp) -> Result<SecretString, OcrError> {
        assert_eq!(op, OcrOp::AcquireAzureAdToken);
        let _count = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(SecretString::from("azure-token"))
    }
}

fn counting_client() -> (OcrClient, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let client = client_with(Arc::new(CountingAzure {
        calls: Arc::clone(&calls),
    }));
    (client, calls)
}

fn counting_host() -> (Arc<dyn OcrHost>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let host: Arc<dyn OcrHost> = Arc::new(CountingHost {
        calls: Arc::clone(&calls),
    });
    (host, calls)
}

struct EnvironmentSecret {
    name: &'static str,
    value: &'static str,
}

impl SecretSource for EnvironmentSecret {
    fn secret(&self, name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        if name == self.name {
            return Ok(Some(SecretString::from(self.value)));
        }
        Ok(None)
    }
}

#[tokio::test]
async fn aocr_returns_the_normalized_mistral_response() {
    let server = MockServer::start(vec![answer_reply()]).await;
    let request = decode_request(wire(
        "mistral-ocr-latest",
        document("data:application/pdf;base64,YWJj"),
        Some(format!("{}/v1", server.base)),
    ))
    .expect("the request decodes");
    let response = aocr(
        ocr_client(OcrSettings::default(), Duration::from_secs(5)),
        request,
    )
    .await
    .expect("the call succeeds");
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    let captured = requests.first().expect("one request was sent");
    let lowered = captured.to_lowercase();
    assert!(lowered.starts_with("post /v1/ocr"), "{lowered}");
    assert!(
        lowered.contains("authorization: bearer sk-test"),
        "{lowered}"
    );
    let fields = captured_body(captured);
    assert_eq!(fields.get("model"), Some(&text("mistral-ocr-latest")));
    assert_eq!(
        fields.get("document"),
        Some(&document("data:application/pdf;base64,YWJj"))
    );
    assert_mistral_response(&response);
}

#[test]
fn an_unsupported_provider_cannot_build_a_call() {
    let Err(error) = LiteLLMOcrRequest::from_inputs(
        "model",
        OcrDocumentInput::from(OcrDocument::DocumentUrl {
            document_url: "https://example.com/a.pdf".to_owned(),
            extra: BTreeMap::new(),
        }),
        Some("anthropic"),
        Object::new(),
        OcrConnectionInputs::default(),
    ) else {
        panic!("the provider is rejected");
    };
    assert!(
        matches!(&error, OcrError::InvalidProvider(provider) if provider == "anthropic"),
        "{error}"
    );
    assert_eq!(error.http_status_code(), Some(400));
}

#[tokio::test]
async fn aocr_reads_a_path_document_in_a_blocking_task() {
    let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir().join(format!("betterlite-ocr-route-{}-{unique}", process::id()));
    fs::create_dir_all(&dir).expect("the directory is created");
    let path = dir.join("scan.pdf");
    fs::write(&path, b"abc").expect("the document is written");
    let server = MockServer::start(vec![answer_reply()]).await;
    let request = decode_request_input(OcrWireRequest::<OcrDocumentInput> {
        model: "mistral-ocr-latest".to_owned(),
        document: OcrDocumentInput::Path {
            path: path.clone(),
            mime_type: None,
        },
        api_key: Some(SecretString::from("sk-test")),
        api_base: Some(format!("{}/v1", server.base)),
        custom_llm_provider: None,
        extra_headers: None,
        optional_params: Object::new(),
        input_sources: BTreeMap::new(),
        timeout_seconds: None,
    })
    .expect("the request decodes");
    let _response = aocr(
        ocr_client(OcrSettings::default(), Duration::from_secs(5)),
        request,
    )
    .await
    .expect("the call succeeds");
    let requests = server.requests().await;
    let lowered = requests
        .first()
        .expect("one request was sent")
        .to_lowercase();
    assert!(
        lowered.contains("data:application/pdf;base64,ywjj"),
        "{lowered}"
    );
    fs::remove_dir_all(&dir).expect("the directory is removed");
}

#[tokio::test]
async fn a_caller_token_call_asks_the_host_once() {
    let server = MockServer::start(vec![answer_reply()]).await;
    let (client, token_calls) = counting_client();
    let (host, host_calls) = counting_host();
    let mut request_wire = wire(
        "azure_ai/pixtral-12b",
        document("data:application/pdf;base64,YWJj"),
        Some(server.base.clone()),
    );
    request_wire.api_key = None;
    let request = decode_request(request_wire).expect("the request decodes");
    let machine = OcrMachine {
        route: OcrRoute { client },
        call: OcrCall {
            request,
            caller_token: true,
        },
        host: Some(host),
    };
    let response = machine.execute(&()).await.expect("the call succeeds");
    assert_eq!(response.model, "pixtral-12b");
    assert_eq!(host_calls.load(Ordering::SeqCst), 1);
    assert_eq!(token_calls.load(Ordering::SeqCst), 1);
    let requests = server.requests().await;
    let lowered = requests
        .first()
        .expect("one request was sent")
        .to_lowercase();
    assert!(
        lowered.contains("authorization: bearer azure-token"),
        "{lowered}"
    );
}

#[tokio::test]
async fn a_call_without_a_caller_token_does_not_ask_the_host() {
    let server = MockServer::start(vec![answer_reply()]).await;
    let (client, _token_calls) = counting_client();
    let (host, host_calls) = counting_host();
    let request = decode_request(wire(
        "azure_ai/pixtral-12b",
        document("data:application/pdf;base64,YWJj"),
        Some(server.base.clone()),
    ))
    .expect("the request decodes");
    let machine = OcrMachine {
        route: OcrRoute { client },
        call: OcrCall {
            request,
            caller_token: false,
        },
        host: Some(host),
    };
    let response = machine.execute(&()).await.expect("the call succeeds");
    assert_eq!(response.model, "pixtral-12b");
    assert_eq!(host_calls.load(Ordering::SeqCst), 0);
    let _requests = server.requests().await;
}

#[tokio::test]
async fn execute_resolves_the_client_secrets_into_the_connection() {
    let server = MockServer::start(vec![answer_reply()]).await;
    let mut client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    client.secrets = Arc::new(EnvironmentSecret {
        name: "AZURE_AI_API_KEY",
        value: "env-key",
    });
    let mut request_wire = wire(
        "azure_ai/pixtral-12b",
        document("data:application/pdf;base64,YWJj"),
        Some(server.base.clone()),
    );
    request_wire.api_key = None;
    let request = decode_request(request_wire).expect("the request decodes");
    let response = OcrRoute { client }
        .execute(request, &())
        .await
        .expect("the call succeeds");
    assert_eq!(response.model, "pixtral-12b");
    let requests = server.requests().await;
    let lowered = requests
        .first()
        .expect("one request was sent")
        .to_lowercase();
    assert!(
        lowered.contains("authorization: bearer env-key"),
        "{lowered}"
    );
}

#[tokio::test]
async fn execute_maps_the_caller_key_through_prepare_secrets() {
    let server = MockServer::start(vec![answer_reply()]).await;
    let mut client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    client.secrets = Arc::new(EnvironmentSecret {
        name: "MISTRAL_AZURE_API_KEY",
        value: "azure-key",
    });
    let mut request_wire = wire(
        "mistral-ocr-latest",
        document("data:application/pdf;base64,YWJj"),
        Some(format!("{}/v1", server.base)),
    );
    request_wire.api_key = None;
    let request = decode_request(request_wire).expect("the request decodes");
    let response = OcrRoute { client }
        .execute(request, &())
        .await
        .expect("the call succeeds");
    assert_eq!(response.model, "mistral-ocr-latest");
    let requests = server.requests().await;
    let lowered = requests
        .first()
        .expect("one request was sent")
        .to_lowercase();
    assert!(
        lowered.contains("authorization: bearer azure-key"),
        "{lowered}"
    );
}

#[tokio::test]
async fn a_caller_token_call_without_a_host_is_invalid() {
    let (client, _token_calls) = counting_client();
    let mut request_wire = wire(
        "azure_ai/pixtral-12b",
        document("data:application/pdf;base64,YWJj"),
        Some("http://127.0.0.1:1".to_owned()),
    );
    request_wire.api_key = None;
    let request = decode_request(request_wire).expect("the request decodes");
    let machine = OcrMachine {
        route: OcrRoute { client },
        call: OcrCall {
            request,
            caller_token: true,
        },
        host: None,
    };
    let error = machine.execute(&()).await.expect_err("the call is refused");
    assert!(matches!(error, OcrError::InvalidRequest(_)), "{error}");
    assert_eq!(error.http_status_code(), Some(400));
}

#[tokio::test]
async fn the_prepare_step_opens_the_litellm_prepare_span() {
    let server = MockServer::start(vec![answer_reply()]).await;
    let request = decode_request(wire(
        "mistral-ocr-latest",
        document("data:application/pdf;base64,YWJj"),
        Some(format!("{}/v1", server.base)),
    ))
    .expect("the request decodes");
    let names = Arc::new(Mutex::new(Vec::new()));
    let subscriber = SpanNames {
        names: Arc::clone(&names),
    };
    let _guard = subscriber::set_default(subscriber);
    let _response = OcrRoute {
        client: ocr_client(OcrSettings::default(), Duration::from_secs(5)),
    }
    .execute(request, &())
    .await
    .expect("the call succeeds");
    let _captured = server.requests().await;
    let recorded = {
        let guard = names.lock().unwrap_or_else(PoisonError::into_inner);
        guard.clone()
    };
    assert!(
        recorded.iter().any(|name| name == "litellm.prepare"),
        "{recorded:?}"
    );
}

struct SpanNames {
    names: Arc<Mutex<Vec<String>>>,
}

impl Subscriber for SpanNames {
    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, span: &Attributes<'_>) -> Id {
        let mut names = self.names.lock().unwrap_or_else(PoisonError::into_inner);
        names.push(span.metadata().name().to_owned());
        Id::from_u64(u64::try_from(names.len()).expect("the id fits"))
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}
