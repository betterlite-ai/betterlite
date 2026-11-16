use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_azure::image_transport::request::{ImageClientConstructor, TimeoutPhases};
use betterlite_azure::image_transport::{ImageReply, ImageRequest, ImageTransport, SystemClock};
use betterlite_azure::images::error::ImageError;
use betterlite_azure::images::generation::{
    GeneratedImage, Generation, Runtime, begin, finish, send,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_config::sections::azure_image_transport::Settings as TransportSettings;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use http::header::HeaderMap;
use secrecy::SecretString;
use serde_json::Value;
use tokio::runtime::Builder;
use tokio::time::Instant;

use super::input::Step;

type GenerationRun = (
    Result<GeneratedImage, ImageError>,
    Vec<RecordedCall>,
    RecordingLogger,
);

fn generation<'a>(
    step: &'a Step,
    api_key: Option<&'a SecretString>,
    token: Option<&'a SecretString>,
) -> Generation<'a> {
    Generation {
        model: step.model.as_deref(),
        prompt: &step.prompt,
        timeout: Some(600.0),
        optional_params: step.optional.clone(),
        headers: step.headers.clone().into_iter().collect(),
        api_key,
        api_base: step.api_base.as_deref(),
        api_version: step.api_version.as_deref(),
        azure_ad_token: token,
        azure_ad_token_provider: None,
        base_model: step.base_model.as_deref(),
        params: LiteLlmParams::default(),
        delegate_async: false,
        now_seconds: 1_700_000_000,
    }
}

pub(super) fn run_generation(step: &Step) -> Option<GenerationRun> {
    let state = Arc::new(FixtureState {
        calls: Mutex::default(),
        reply: Mutex::new(step.reply_body.clone()),
    });
    let transport = FixtureTransport {
        state: Arc::clone(&state),
    };
    let mut logger = RecordingLogger::default();
    let endpoints = EndpointSettings::default();
    let polling_timeout = TransportSettings::default().operation_polling_timeout;
    let api_key = step.api_key.as_deref().map(SecretString::from);
    let token = step.azure_ad_token.as_deref().map(SecretString::from);
    let call = generation(step, api_key.as_ref(), token.as_ref());
    let clock = SystemClock {
        origin: Instant::now(),
    };
    let environment = BTreeMap::<String, String>::new();
    let sources = NoSources;
    let mut runtime = Runtime {
        endpoints: &endpoints,
        environment: &environment,
        sources: &sources,
        refresh_enabled: false,
        polling_timeout,
        constructor: &transport,
        clock: &clock,
        logger: &mut logger,
    };
    let result = exchange(call, &mut runtime)?;
    let calls = state
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Some((result, calls, logger))
}

// the three production steps around the one await, driven by a current-thread runtime
fn exchange(
    call: Generation<'_>,
    runtime: &mut Runtime<'_, FixtureTransport, SystemClock>,
) -> Option<Result<GeneratedImage, ImageError>> {
    let driver = Builder::new_current_thread().enable_all().build().ok()?;
    Some(match begin(call, runtime) {
        Ok(begun) => {
            let sent = driver.block_on(send(
                &begun,
                runtime.constructor,
                runtime.clock,
                runtime.polling_timeout,
            ));
            finish(&begun, sent, runtime.logger)
        }
        Err(error) => Err(error),
    })
}

struct FixtureState {
    calls: Mutex<Vec<RecordedCall>>,
    reply: Mutex<String>,
}

#[derive(Clone)]
pub(super) struct RecordedCall {
    pub(super) url: String,
    pub(super) body: Option<Value>,
    // only the auth headers travel back, since httpx adds its own defaults
    pub(super) headers: BTreeMap<String, Option<String>>,
}

pub(super) struct FixtureTransport {
    state: Arc<FixtureState>,
}

impl ImageTransport for FixtureTransport {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<
        Output = Result<ImageReply, betterlite_http::proxy::transport_error::TransportError>,
    > + Send {
        let mut headers = BTreeMap::new();
        for name in ["api-key", "authorization"] {
            let value = request
                .headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            headers.insert(name.to_owned(), value);
        }
        self.state
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                url: request.url.clone(),
                body: serde_json::from_slice(&request.body).ok(),
                headers,
            });
        let body = self
            .state
            .reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        ready(Ok(ImageReply {
            status: 200,
            headers: HeaderMap::new(),
            body: body.into_bytes(),
        }))
    }
}

impl ImageClientConstructor for FixtureTransport {
    type Client = Self;

    fn construct(&self, _timeout: TimeoutPhases) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

#[derive(Default)]
pub(super) struct RecordingLogger {
    pub(super) pre: Vec<PreCallRecord>,
    pub(super) post: Vec<PostCallRecord>,
}

impl ImageGenerationLogger for RecordingLogger {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(record);
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(record);
    }
}

type Source<'source> = Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

pub(super) struct NoSources;

impl CredentialSources for NoSources {
    fn client_secret<'a>(&'a self, _record: &ClientSecretRecord) -> Source<'a> {
        Err(unavailable("client secret"))
    }

    fn username_password<'a>(&'a self, _record: &UsernamePasswordRecord) -> Source<'a> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'a>(&'a self, _scope: &str) -> Source<'a> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'a>(
        &'a self,
        _scope: &'a str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'a>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}
