use std::collections::BTreeMap;
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_azure::image_transport::request::{ImageClientConstructor, TimeoutPhases};
use betterlite_azure::image_transport::{ImageClock, ImageReply, ImageRequest, ImageTransport};
use betterlite_azure::images::edit::{
    EditAuth, EditParams, EditUrl, finalize_image_edit_request_data, get_complete_url,
    validate_environment,
};
use betterlite_azure::images::error::{ClassifiedError, ImageError};
use betterlite_azure::images::generation::{GeneratedImage, Generation, Runtime, image_generation};
use betterlite_azure::images::mai::{MaiProjection, MaiReply};
use betterlite_azure::images::selection::get_azure_image_generation_config;
use betterlite_azure::images::target::{AzureClientParams, create_azure_base_url};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::azure_url_policy::AzureCredentials;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::image_generation::logging::{
    ImageGenerationLogger, PostCallRecord, PreCallRecord,
};
use betterlite_openai::image_generation::reply::ImageGenerationReply;
use http::header::HeaderMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::helper::HelperResult;

use super::input::{AuthStep, FormStep, GenerationStep, Input, TargetStep, UrlStep};

pub(crate) fn run(input: &Input) -> HelperResult {
    let models: Vec<Value> = input
        .models
        .iter()
        .map(|model| json!({"class": get_azure_image_generation_config(Some(model)).class_name()}))
        .collect();
    Ok(json!({
        "targets": input.targets.iter().map(target_value).collect::<Vec<_>>(),
        "models": models,
        "forms": input.forms.iter().map(form_value).collect::<Vec<_>>(),
        "urls": input.urls.iter().map(url_value).collect::<Vec<_>>(),
        "auth": input.auth.iter().map(auth_value).collect::<Vec<_>>(),
        "generations": input.generations.iter().map(generation_value).collect::<Vec<_>>(),
    }))
}

fn target_value(step: &TargetStep) -> Value {
    let outcome = create_azure_base_url(
        &AzureClientParams {
            azure_endpoint: step.azure_endpoint.as_deref(),
            base_url: step.base_url.as_deref(),
            api_version: step.api_version.as_deref(),
        },
        step.model.as_deref(),
        step.base_model.as_deref(),
    );
    match outcome {
        Ok(url) => json!({"url": url}),
        Err(error) => failure(&error),
    }
}

fn form_value(step: &FormStep) -> Value {
    let data = finalize_image_edit_request_data(&step.data, &step.request_url);
    json!({"fields": serde_json::to_value(data).unwrap_or(Value::Null)})
}

fn url_value(step: &UrlStep) -> Value {
    let credentials = AzureCredentials {
        api_base: step.global_base.clone(),
        api_version: step.global_version.clone(),
        api_key: None,
    };
    match get_complete_url(&EditUrl {
        model: &step.model,
        api_base: step.api_base.as_deref(),
        api_version: step.api_version.as_deref(),
        credentials: &credentials,
        default_api_version: "2025-02-01-preview",
    }) {
        Ok(url) => json!({"url": url}),
        Err(error) => failure(&error),
    }
}

fn auth_value(step: &AuthStep) -> Value {
    let environment: BTreeMap<String, String> = step
        .env_token
        .iter()
        .map(|token| ("AZURE_AD_TOKEN".to_owned(), token.clone()))
        .collect();
    let params = EditParams {
        api_key: step.params.api_key.as_deref().map(SecretString::from),
        azure_ad_token: step
            .params
            .azure_ad_token
            .as_deref()
            .map(SecretString::from),
        auth: betterlite_azure::azure_urls::AuthParams {
            tenant_id: step.params.auth.tenant_id.clone(),
            client_id: step.params.auth.client_id.clone(),
            client_secret: step
                .params
                .auth
                .client_secret
                .as_deref()
                .map(SecretString::from),
            azure_username: step.params.auth.azure_username.clone(),
            azure_password: step
                .params
                .auth
                .azure_password
                .as_deref()
                .map(SecretString::from),
            azure_scope: step.params.auth.azure_scope.clone(),
        },
    };
    match validate_environment(&EditAuth {
        headers: step.headers.clone(),
        api_key: step.api_key.as_deref(),
        params: &params,
        credentials: &AzureCredentials::default(),
        environment: &environment,
        sources: &NoSources,
        refresh_enabled: false,
        provider: None,
    }) {
        Ok(headers) => json!({"headers": serde_json::to_value(headers).unwrap_or(Value::Null)}),
        Err(error) => failure(&error),
    }
}

fn failure(error: &dyn Display) -> Value {
    json!({
        "kind": "error",
        "status": Value::Null,
        "type": Value::Null,
        "param": Value::Null,
        "code": Value::Null,
        "message": error.to_string(),
    })
}

fn generation_value(step: &GenerationStep) -> Value {
    match run_generation(step) {
        Some((result, calls, logger)) => generation_report(&result, &calls, &logger),
        None => failure(&"the fixture config does not load"),
    }
}

type GenerationOutcome = (
    Result<GeneratedImage, ImageError>,
    Vec<RecordedCall>,
    RecordingLogger,
);

fn run_generation(step: &GenerationStep) -> Option<GenerationOutcome> {
    let state = Arc::new(FixtureState {
        calls: Mutex::default(),
        reply: Mutex::new(Some((step.reply.status, step.reply.body.clone()))),
    });
    let constructor = Constructor {
        state: Arc::clone(&state),
    };
    let clock = FixtureClock;
    let mut logger = RecordingLogger::default();
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("");
    let config = config.ok()?;
    let api_key = step.api_key.as_deref().map(SecretString::from);
    let token = step.azure_ad_token.as_deref().map(SecretString::from);
    let call = Generation {
        model: step.model.as_deref(),
        prompt: &step.prompt,
        timeout: None,
        optional_params: step.optional.clone(),
        headers: step
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        api_key: api_key.as_ref(),
        api_base: step.api_base.as_deref(),
        api_version: step.api_version.as_deref(),
        azure_ad_token: token.as_ref(),
        azure_ad_token_provider: None,
        base_model: step.base_model.as_deref(),
        params: LiteLlmParams::default(),
        delegate_async: step.is_async,
        now_seconds: 1_700_000_000,
    };
    let mut runtime = Runtime {
        endpoints: &config.sections.azure_endpoints,
        environment: &BTreeMap::<String, String>::new(),
        sources: &NoSources,
        refresh_enabled: false,
        polling_timeout: config
            .sections
            .azure_image_transport
            .operation_polling_timeout,
        constructor: &constructor,
        clock: &clock,
        logger: &mut logger,
        mai: &MissingMai,
    };
    let driver = Builder::new_current_thread().enable_all().build().ok()?;
    let result = driver.block_on(image_generation(call, &mut runtime));
    let calls = lock(&state.calls).clone();
    Some((result, calls, logger))
}

fn generation_report(
    result: &Result<GeneratedImage, ImageError>,
    calls: &[RecordedCall],
    logger: &RecordingLogger,
) -> Value {
    let pre_calls = logger.pre.len();
    let post_calls = logger.post.len();
    match result {
        Ok(image) => {
            let first = calls.first();
            json!({
                "kind": "response",
                "sent_url": first.map(|call| call.url.clone()),
                "sent_body": first.and_then(|call| call.body.clone()),
                "sent_headers": first.map(|call| serde_json::to_value(&call.headers).unwrap_or(Value::Null)),
                "input": logger.pre.first().map(|record| record.input.clone()),
                "pre_calls": pre_calls,
                "post_calls": post_calls,
                "reply": reply_value(&image.reply),
            })
        }
        Err(error) => {
            let sent = calls.first();
            json!({
                "kind": "error",
                "status": error.status(),
                "type": Value::Null,
                "param": Value::Null,
                "code": Value::Null,
                "message": error.to_string(),
                "sent_url": sent.map(|call| call.url.clone()),
                "sent_body": sent.and_then(|call| call.body.clone()),
                "pre_calls": pre_calls,
                "post_calls": post_calls,
            })
        }
    }
}

fn reply_value(reply: &ImageGenerationReply) -> Value {
    let first = reply.data.first();
    json!({
        "created": reply.created,
        "size": reply.size,
        "quality": reply.quality,
        "output_format": reply.output_format,
        "b64_json": first.and_then(|item| item.b64_json.clone()),
        "image_url": first.and_then(|item| item.url.clone()),
        "images": reply.data.len(),
    })
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct FixtureState {
    calls: Mutex<Vec<RecordedCall>>,
    reply: Mutex<Option<(u16, String)>>,
}

#[derive(Clone)]
pub(crate) struct RecordedCall {
    pub(crate) url: String,
    pub(crate) body: Option<Value>,
    // only the auth headers travel back, since httpx adds its own defaults
    pub(crate) headers: BTreeMap<String, Option<String>>,
}

pub(crate) struct FixtureTransport {
    state: Arc<FixtureState>,
}

impl ImageTransport for FixtureTransport {
    fn send(
        &self,
        request: ImageRequest,
    ) -> impl Future<
        Output = Result<ImageReply, betterlite_http::proxy::transport_error::TransportError>,
    > + Send {
        let headers = auth_headers(&request.headers);
        lock(&self.state.calls).push(RecordedCall {
            url: request.url.clone(),
            body: serde_json::from_slice(&request.body).ok(),
            headers,
        });
        let reply = lock(&self.state.reply)
            .take()
            .unwrap_or((200, String::from("{}")));
        ready(Ok(ImageReply {
            status: reply.0,
            headers: HeaderMap::new(),
            body: reply.1.into_bytes(),
        }))
    }
}

fn auth_headers(headers: &HeaderMap) -> BTreeMap<String, Option<String>> {
    let mut found = BTreeMap::new();
    for name in ["api-key", "authorization"] {
        found.insert(
            name.to_owned(),
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        );
    }
    found
}

pub(crate) struct Constructor {
    pub(crate) state: Arc<FixtureState>,
}

impl ImageClientConstructor for Constructor {
    type Client = FixtureTransport;

    fn construct(&self, _timeout: TimeoutPhases) -> FixtureTransport {
        FixtureTransport {
            state: Arc::clone(&self.state),
        }
    }
}

pub(crate) struct FixtureClock;

impl ImageClock for FixtureClock {
    fn now(&self) -> Duration {
        Duration::from_secs(0)
    }

    fn pause(&self, _seconds: i64) -> impl Future<Output = ()> + Send {
        ready(())
    }
}

#[derive(Default)]
pub(crate) struct RecordingLogger {
    pub(crate) pre: Vec<PreCallRecord>,
    pub(crate) post: Vec<PostCallRecord>,
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

// B554 owns the MAI reply; the Azure adapter crate cannot depend on betterlite-foundry
pub(crate) struct MissingMai;

impl MaiProjection for MissingMai {
    fn project(
        &self,
        _reply: MaiReply<'_>,
        _logger: &mut dyn ImageGenerationLogger,
    ) -> Result<ImageGenerationReply, ClassifiedError> {
        Err(ClassifiedError {
            class: String::from("NotBuilt"),
            status: None,
            message: String::from("the MAI reply is B554"),
        })
    }
}

pub(crate) struct NoSources;

fn unavailable(kind: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: format!("no {kind} source is configured"),
    }
}

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &betterlite_azure::azure_default_chain::grant::ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("client secret"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &betterlite_azure::azure_default_chain::grant::UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("username and password"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(unavailable("SDK refresh"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(unavailable("OIDC"))
    }
}
