use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::io::Write as _;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::vector_store_http_write::service::{
    ClientKind, LogContext, SuppliedClient,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::request::{Request, read_request};
use betterlite_http_fixture::server::READ_TIMEOUT;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::header::HeaderMap;
use secrecy::SecretString;

use crate::api::transcription_contract::{AudioFile, AudioTranscriptionConfig as _};
use crate::api::transcription_http::config::Headers;
use crate::api::transcription_http::reply::TranscriptionReply;
use crate::watsonx_transcription::config::BoundConfig;
use crate::watsonx_transcription::error::WatsonxTranscriptionError;
use crate::watsonx_transcription::{WatsonxTranscription, WatsonxTranscriptionCall};

pub(crate) const MODEL: &str = "granite-4-0-8b";
pub(crate) const AUDIO: &str = "RIFFwave-bytes";
pub(crate) const SUPPLIED_BODY_BYTES: usize = 8;
const AUDIO_NAME: &str = "clip.wav";
const AUDIO_TYPE: &str = "audio/wav";

// The supplied client's own limit, so a reply readable only through the pooled client names which one sent
const SUPPLIED_LIMITS: TransportLimits = TransportLimits {
    body_bytes: SUPPLIED_BODY_BYTES,
    ..TransportLimits::DEFAULT
};

// A bounded recording server: every connection is read, recorded and answered, so a second request shows up
pub(crate) struct Recorder {
    pub(crate) address: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Recorder {
    pub(crate) fn start(answer: Vec<u8>, pause: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the recorder binds");
        let address = listener.local_addr().expect("the recorder has an address");
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let stop: Arc<AtomicBool> = Arc::default();
        let recorded = Arc::clone(&requests);
        let ending = Arc::clone(&stop);
        let answer = Arc::new(answer);
        let handle = thread::Builder::new()
            .name(String::from("watsonx-transcription"))
            .spawn(move || record(&listener, &answer, pause, &recorded, &ending))
            .expect("the recorder starts");
        Self {
            address,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    pub(crate) fn finish(mut self) -> Vec<Request> {
        self.stop.store(true, Ordering::SeqCst);
        drop(TcpStream::connect(self.address));
        if let Some(handle) = self.handle.take() {
            handle.join().expect("the recorder stops");
        }
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut *requests)
    }
}

fn record(
    listener: &TcpListener,
    answer: &[u8],
    pause: Duration,
    requests: &Mutex<Vec<Request>>,
    stop: &AtomicBool,
) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if stream.set_read_timeout(Some(READ_TIMEOUT)).is_err() {
            continue;
        }
        let Ok(request) = read_request(&mut stream) else {
            continue;
        };
        requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        thread::park_timeout(pause);
        if stream.write_all(answer).is_err() {
            continue;
        }
    }
}

// The IAM exchange a token in the environment must never reach
pub(crate) struct Iam {
    answer: Option<&'static str>,
    pub(crate) calls: Arc<Mutex<usize>>,
}

impl Iam {
    pub(crate) fn refusing() -> Self {
        Self {
            answer: None,
            calls: Arc::default(),
        }
    }

    pub(crate) fn answering(body: &'static str) -> Self {
        Self {
            answer: Some(body),
            calls: Arc::default(),
        }
    }
}

impl WatsonxTransport for Iam {
    fn send_buffered(
        &self,
        _request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        *calls = calls.saturating_add(1);
        drop(calls);
        let body = self.answer.map(|body| body.as_bytes().to_vec());
        ready(body.map_or_else(
            || Err(TransportError::Closed),
            |body| {
                Ok(UpstreamResponse {
                    status: 200_u16,
                    headers: HeaderMap::new(),
                    body,
                })
            },
        ))
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// The same logging state a dispatched request carries into the executor's pre-call
pub(crate) struct Logging {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Logging {
    pub(crate) fn new() -> Self {
        let init: LoggingInit = serde_json::from_str(
            r#"{"model":"granite","call_type":"transcription","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
        )
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
        }
    }

    pub(crate) async fn transcribe<T: WatsonxTransport>(
        &mut self,
        auth: &WatsonxAuth<T>,
        client: &ProxiedClient,
        call: &WatsonxTranscriptionCall<'_>,
    ) -> Result<TranscriptionReply, WatsonxTranscriptionError> {
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        };
        let mut log = LogContext {
            state: &mut self.state,
            env: &env,
        };
        WatsonxTranscription {
            auth,
            client,
            log: &mut log,
        }
        .transcribe(call)
        .await
    }
}

pub(crate) struct Outcome {
    pub(crate) requests: Vec<Request>,
    pub(crate) result: Result<TranscriptionReply, WatsonxTranscriptionError>,
    pub(crate) iam_calls: usize,
}

pub(crate) struct Run {
    pub(crate) optional: Object,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) asynchronous: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) limits: TransportLimits,
    pub(crate) pause: Duration,
    pub(crate) supplied: bool,
    pub(crate) answer: Vec<u8>,
}

impl Run {
    pub(crate) fn new(answer: Vec<u8>) -> Self {
        Self {
            optional: Object::new(),
            api_key: None,
            asynchronous: false,
            timeout: None,
            limits: TransportLimits::DEFAULT,
            pause: Duration::ZERO,
            supplied: false,
            answer,
        }
    }

    pub(crate) async fn execute(self, resolved: WatsonxEnvironment, iam: Iam) -> Outcome {
        let (recorder, pooled, supplied_client) = self.clients();
        let calls = Arc::clone(&iam.calls);
        let auth = WatsonxAuth::new(resolved, iam, Arc::new(|| 0.0_f64)).expect("the auth builds");
        let api_base = format!("http://{}", recorder.address);
        let empty = Object::new();
        let call = WatsonxTranscriptionCall {
            model: MODEL,
            audio_file: AudioFile {
                filename: AUDIO_NAME,
                content: AUDIO.as_bytes(),
                content_type: AUDIO_TYPE,
            },
            api_key: self.api_key.as_ref(),
            api_base: Some(&api_base),
            optional_params: &self.optional,
            litellm_params: &empty,
            timeout: self.timeout,
            asynchronous: self.asynchronous,
            supplied: supplied_client.as_ref().map(|client| SuppliedClient {
                kind: ClientKind::Sync,
                transport: client,
            }),
            shared_session: None,
        };
        let mut logging = Logging::new();
        let result = logging.transcribe(&auth, &pooled, &call).await;
        Outcome {
            requests: recorder.finish(),
            result,
            iam_calls: *calls.lock().unwrap_or_else(PoisonError::into_inner),
        }
    }

    fn clients(&self) -> (Recorder, ProxiedClient, Option<ProxiedClient>) {
        let recorder = Recorder::start(self.answer.clone(), self.pause);
        let pooled = proxied(self.limits);
        let supplied = self.supplied.then(|| proxied(SUPPLIED_LIMITS));
        (recorder, pooled, supplied)
    }
}

pub(crate) fn proxied(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(base_builder().http1_only(), &policy, limits)
        .expect("the proxied client builds")
}

// The options B5842's transform may carry into the form, read through the adapter the operation builds
pub(crate) fn probe_supported_options(resolved: WatsonxEnvironment) -> Vec<String> {
    let auth =
        WatsonxAuth::new(resolved, Iam::refusing(), Arc::new(|| 0.0_f64)).expect("the auth builds");
    BoundConfig::new(&auth, Headers::new()).supported_openai_params(MODEL)
}

pub(crate) fn environment(values: &[(&str, &str)]) -> WatsonxEnvironment {
    let find = |name: &str| {
        values
            .iter()
            .find(|(key, _value)| *key == name)
            .map(|(_key, value)| *value)
    };
    let secret = |name: &str| find(name).map(|value| SecretString::from(value.to_owned()));
    let text = |name: &str| find(name).map(str::to_owned);
    WatsonxEnvironment {
        watsonx_iam_url: text("WATSONX_IAM_URL"),
        wx_api_key: secret("WX_API_KEY"),
        watsonx_api_key: secret("WATSONX_API_KEY"),
        watsonx_apikey: secret("WATSONX_APIKEY"),
        watsonx_zenapikey: secret("WATSONX_ZENAPIKEY"),
        watsonx_token: secret("WATSONX_TOKEN"),
        watsonx_api_base: text("WATSONX_API_BASE"),
        watsonx_url: text("WATSONX_URL"),
        wx_url: text("WX_URL"),
        wml_url: text("WML_URL"),
        watsonx_project_id: text("WATSONX_PROJECT_ID"),
        wx_project_id: text("WX_PROJECT_ID"),
        project_id: text("PROJECT_ID"),
        watsonx_region: text("WATSONX_REGION"),
        wx_region: text("WX_REGION"),
        region: text("REGION"),
        watsonx_deployment_space_id: text("WATSONX_DEPLOYMENT_SPACE_ID"),
        watsonx_space_id: text("WATSONX_SPACE_ID"),
        wx_space_id: text("WX_SPACE_ID"),
        space_id: text("SPACE_ID"),
    }
}

pub(crate) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture is valid")
}
