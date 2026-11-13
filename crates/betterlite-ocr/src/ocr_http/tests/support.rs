use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::str;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::json::{Json, Object};
use crate::ocr_auth::{
    AwsRequestSigner, AwsSigningRequest, AzureTokenSource, VertexAccessToken, VertexTokenSource,
};
use crate::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use crate::ocr_config::decode::decode_and_normalize_response;
use crate::ocr_config::{
    OCR_RESPONSE_MAX_BYTES, OcrProviderConfig, OcrTransform, PreparedOcrRequest, Secrets,
};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrError, OcrSecretError};
use crate::ocr_http::{OcrClient, SecretSource};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;
use crate::response::{OCRPage, OCRResponse};

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) bytes: Vec<u8>,
    delay: Duration,
}

pub(crate) fn reply(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Reply {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    Reply {
        bytes,
        delay: Duration::ZERO,
    }
}

pub(crate) fn chunked(chunks: &[&[u8]]) -> Reply {
    let mut body = Vec::new();
    for chunk in chunks {
        body.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        body.extend_from_slice(chunk);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"0\r\n\r\n");
    let head = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
    let mut bytes = head.as_bytes().to_vec();
    bytes.extend_from_slice(&body);
    Reply {
        bytes,
        delay: Duration::ZERO,
    }
}

pub(crate) fn delayed(mut reply: Reply, delay: Duration) -> Reply {
    reply.delay = delay;
    reply
}

pub(crate) struct MockServer {
    pub(crate) base: String,
    tasks: JoinSet<Vec<String>>,
}

impl MockServer {
    pub(crate) async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("the mock server binds");
        let address = listener
            .local_addr()
            .expect("the mock server has an address");
        let mut tasks = JoinSet::new();
        tasks.spawn(serve(listener, replies));
        Self {
            base: format!("http://{address}"),
            tasks,
        }
    }

    pub(crate) async fn requests(mut self) -> Vec<String> {
        self.tasks
            .join_next()
            .await
            .expect("the server task runs")
            .expect("the server task does not panic")
    }
}

async fn serve(listener: TcpListener, replies: Vec<Reply>) -> Vec<String> {
    let mut requests = Vec::new();
    for reply in replies {
        let (mut socket, _) = listener.accept().await.expect("the mock server accepts");
        requests.push(read_request(&mut socket).await);
        if !reply.delay.is_zero() {
            sleep(reply.delay).await;
        }
        socket
            .write_all(&reply.bytes)
            .await
            .expect("the mock server writes");
        socket.shutdown().await.expect("the mock server closes");
    }
    requests
}

async fn read_request(socket: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 2048];
    loop {
        let read = socket
            .read(&mut chunk)
            .await
            .expect("the mock server reads the request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
            && buffer.len()
                >= end
                    .saturating_add(4)
                    .saturating_add(content_length(&buffer[..end]))
        {
            break;
        }
    }
    String::from_utf8(buffer).expect("the request is UTF-8")
}

fn content_length(head: &[u8]) -> usize {
    let text = str::from_utf8(head).expect("the request head is UTF-8");
    text.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("a numeric length"))
        })
        .unwrap_or(0)
}

pub(crate) fn http_client(read_timeout: Duration) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the test proxy policy resolves"),
    );
    let limits = TransportLimits {
        read_timeout,
        ..TransportLimits::DEFAULT
    };
    ProxiedClient::build(base_builder(), &policy, limits).expect("the test client builds")
}

pub(crate) fn ocr_client(settings: OcrSettings, read_timeout: Duration) -> OcrClient {
    let allowed = vec!["127.0.0.1".to_owned()];
    let validator = UrlValidator::new(SystemResolver, &allowed, &Verification::Enabled);
    OcrClient {
        provider_http: http_client(read_timeout),
        polling_http: http_client(read_timeout),
        document_http: http_client(read_timeout),
        document_fetcher: UserUrlFetcher {
            validator,
            validation: true,
        },
        settings,
        secrets: Arc::new(NoSecrets),
        azure: Arc::new(NoAzure),
        aws: Arc::new(NoAws),
        vertex: Arc::new(NoVertex),
    }
}

pub(crate) fn connection(settings: OcrSettings) -> OcrConnection {
    OcrConnection::new(
        ResolvedOcrCredentials::default(),
        OcrTransportConfig::default(),
        settings,
        Secrets::new(),
    )
}

pub(crate) fn prepared_request(api_base: String, optional_params: Object) -> PreparedOcrRequest {
    let credentials = ResolvedOcrCredentials {
        api_key: None,
        api_base: Some(OcrSourced {
            value: api_base,
            source: OcrInputSource::Request,
        }),
    };
    let transport = OcrTransportConfig {
        extra_headers: Vec::new(),
        extra_headers_source: OcrInputSource::Deployment,
        timeout: None,
        max_response_bytes: OCR_RESPONSE_MAX_BYTES,
    };
    PreparedOcrRequest {
        model: "requested-model".to_owned(),
        document: OcrDocument::DocumentUrl {
            document_url: "data:application/pdf;base64,YWJj".to_owned(),
            extra: BTreeMap::new(),
        },
        connection: OcrConnection::new(
            credentials,
            transport,
            OcrSettings::default(),
            Secrets::new(),
        ),
        caller_document: false,
        optional_params,
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    }
}

struct NoSecrets;

impl SecretSource for NoSecrets {
    fn secret(&self, _name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        Ok(None)
    }
}

struct NoAzure;

#[async_trait]
impl AzureTokenSource for NoAzure {
    async fn token(
        &self,
        _request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        Ok(None)
    }
}

struct NoAws;

#[async_trait]
impl AwsRequestSigner for NoAws {
    async fn sign(
        &self,
        _request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError> {
        Ok(Vec::new())
    }
}

struct NoVertex;

#[async_trait]
impl VertexTokenSource for NoVertex {
    async fn token(&self, _request: &PreparedOcrRequest) -> Result<VertexAccessToken, OcrError> {
        Err(OcrError::Unsupported("vertex tokens in this test"))
    }
}

#[derive(Serialize)]
pub(crate) struct TestBody {
    pub(crate) model: String,
    pub(crate) document: OcrDocument,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TestPayload {
    pub(crate) pages: Vec<TestPage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TestPage {
    pub(crate) index: i64,
    pub(crate) markdown: String,
}

#[derive(Clone, Copy)]
pub(crate) struct TestConfig;

impl OcrProviderConfig for TestConfig {
    type OcrParams = ();
    type ProviderRequest = TestBody;
    type Environment = Vec<(String, String)>;

    fn secret_names(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn get_supported_ocr_params(&self, _model: &str) -> &'static [&'static str] {
        &["pages"]
    }

    fn map_ocr_params(
        &self,
        _non_default_params: &Object,
        _model: &str,
    ) -> Result<Self::OcrParams, OcrError> {
        Ok(())
    }

    fn validate_environment(
        &self,
        _request: &PreparedOcrRequest,
        _client: &OcrClient,
    ) -> impl Future<Output = Result<Self::Environment, OcrError>> + Send {
        ready(Ok(vec![(
            "authorization".to_owned(),
            "Bearer test-key".to_owned(),
        )]))
    }

    fn get_complete_url(
        &self,
        request: &PreparedOcrRequest,
        _optional_params: &Self::OcrParams,
        _environment: &Self::Environment,
    ) -> Result<String, OcrError> {
        let base = request.connection.api_base.as_deref().unwrap_or_default();
        Ok(format!("{base}/v1/ocr"))
    }

    fn transform_ocr_request(
        &self,
        document: OcrDocument,
        transform: OcrTransform<'_, Self::OcrParams>,
    ) -> Result<Self::ProviderRequest, OcrError> {
        Ok(TestBody {
            model: transform.model.to_owned(),
            document,
        })
    }

    fn transform_ocr_response(
        &self,
        model: &str,
        raw_response: &[u8],
        request_format: RequestFormat,
    ) -> Result<OCRResponse, OcrError> {
        decode_and_normalize_response(
            model,
            raw_response,
            request_format,
            |request_model, payload: TestPayload| {
                Ok(OCRResponse {
                    pages: payload.pages.into_iter().map(page).collect(),
                    model: request_model.to_owned(),
                    document_annotation: None,
                    usage_info: None,
                    content: None,
                    tables: None,
                    key_value_pairs: None,
                    object: "ocr".to_owned(),
                    extra: Object::new(),
                    provider_native_response: None,
                })
            },
        )
    }
}

fn page(page: TestPage) -> OCRPage {
    OCRPage {
        index: page.index,
        markdown: page.markdown,
        images: None,
        dimensions: None,
        extra: Object::new(),
    }
}

pub(crate) fn object(entries: &[(&str, Json)]) -> Object {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}
