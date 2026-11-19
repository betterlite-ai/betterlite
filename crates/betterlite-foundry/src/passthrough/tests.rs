#![cfg(test)]

mod passthrough_cases;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_azure::passthrough::logging::{
    ChatReplyProjection, ChatStreamAssembly, NoChatReplyProjection, NoChatStreamAssembly,
};
use betterlite_ocr::ocr_error::OcrError;
use betterlite_ocr::response::OCRResponse;
use betterlite_openai::chat_choices::reply::ResponseReply;
use betterlite_openai::compatible_wire::LogInput;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughTarget, RelayCallType,
};
use betterlite_prompts::arguments::json::Object as ParamsObject;
use secrecy::{ExposeSecret as _, SecretString};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::credentials::error::CredentialsError;

use super::auth::{ValidateEnvironmentRequest, validate_environment};
use super::error::FoundryPassthroughError;
use super::logging::FoundryPassthroughLogging;
use super::ocr::{NativeOcrPassthrough, decode_logged_ocr_reply};
use super::plan::FoundryPassthroughPlan;
use super::url::foundry_root;

pub(super) type Plan = FoundryPassthroughPlan<NoChatReplyProjection, NoChatStreamAssembly>;

pub(super) const OCR_MODEL: &str = "azure_ai/mistral-ocr-latest";
pub(super) const OCR_ENDPOINT: &str = "providers/mistral/azure/ocr";

#[derive(Debug, Default)]
pub(super) struct RecordingLogging {
    pub(super) call_type: Option<String>,
}

impl PassthroughLogging for RecordingLogging {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.call_type = Some(call_type.as_str().to_owned());
    }
}

impl FoundryPassthroughLogging for RecordingLogging {
    fn set_ocr_call_type(&mut self) {
        self.call_type = Some(String::from("aocr"));
    }
}

#[derive(Debug, Clone)]
pub(super) struct RecordedOcrCall {
    pub(super) model: String,
    pub(super) endpoint: String,
    pub(super) body: Vec<u8>,
}

pub(super) enum OcrScript {
    Response(OCRResponse),
    Absent,
    Failure,
}

#[derive(Debug, Default)]
pub(super) struct RecordingOcr {
    pub(super) script: Option<OcrScript>,
    pub(super) calls: Mutex<Vec<RecordedOcrCall>>,
}

impl RecordingOcr {
    pub(super) fn scripted(script: OcrScript) -> Arc<Self> {
        Arc::new(Self {
            script: Some(script),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<RecordedOcrCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl NativeOcrPassthrough for RecordingOcr {
    fn passthrough_response(
        &self,
        model: &str,
        endpoint: &str,
        body: &[u8],
    ) -> Result<Option<OCRResponse>, OcrError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedOcrCall {
                model: model.to_owned(),
                endpoint: endpoint.to_owned(),
                body: body.to_vec(),
            });
        match &self.script {
            Some(OcrScript::Response(response)) => Ok(Some(response.clone())),
            Some(OcrScript::Absent) | None => Ok(None),
            Some(OcrScript::Failure) => {
                Err(OcrError::InvalidRequest(String::from("malformed OCR body")))
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct RecordingChat {
    pub(super) calls: Arc<Mutex<Vec<Object>>>,
}

impl ChatReplyProjection for RecordingChat {
    fn project_chat_reply(
        &self,
        _model: &str,
        _reply: &ResponseReply<'_>,
        _input: LogInput,
    ) -> Option<Object> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Object::new());
        Some(Object::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedStreamCall {
    pub(super) chunks: usize,
    pub(super) model: String,
    pub(super) messages: Option<usize>,
}

#[derive(Debug, Default)]
pub(super) struct RecordingStream {
    pub(super) calls: Arc<Mutex<Vec<RecordedStreamCall>>>,
}

impl ChatStreamAssembly for RecordingStream {
    fn complete_streaming_response(
        &self,
        chunks: &[String],
        model: &str,
        messages: Option<&[Object]>,
    ) -> Option<Object> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedStreamCall {
                chunks: chunks.len(),
                model: model.to_owned(),
                messages: messages.map(<[Object]>::len),
            });
        Some(Object::new())
    }
}

#[derive(Clone, Default)]
pub(super) struct Warnings(Arc<Mutex<Vec<String>>>);

impl Warnings {
    pub(super) fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = MessageField::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(super) fn capture_warnings<T, Run>(run: Run) -> (T, Vec<String>)
where
    Run: FnOnce() -> T,
{
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    let value = tracing::subscriber::with_default(subscriber, run);
    (value, warnings.messages())
}

pub(super) fn build_plan(
    global_api_base: Option<&str>,
    global_api_key: Option<&str>,
    api_base_env: Option<&str>,
    api_key_env: Option<&str>,
    litellm_params: Object,
) -> Plan {
    Plan {
        global_api_base: global_api_base.map(str::to_owned),
        global_api_key: global_api_key.map(SecretString::from),
        azure: AzurePassthroughConfig {
            api_base_env: api_base_env.map(str::to_owned),
            api_key_env: api_key_env.map(SecretString::from),
            model_call_details: None,
            chat_reply: NoChatReplyProjection,
            chat_stream: NoChatStreamAssembly,
        },
        ocr: None,
        litellm_params,
    }
}

pub(super) fn plan_with_reply<Reply>(
    chat_reply: Reply,
) -> FoundryPassthroughPlan<Reply, NoChatStreamAssembly>
where
    Reply: ChatReplyProjection,
{
    FoundryPassthroughPlan {
        global_api_base: None,
        global_api_key: None,
        azure: AzurePassthroughConfig {
            api_base_env: None,
            api_key_env: None,
            model_call_details: None,
            chat_reply,
            chat_stream: NoChatStreamAssembly,
        },
        ocr: None,
        litellm_params: Object::new(),
    }
}

pub(super) fn plan_with_stream<Stream>(
    chat_stream: Stream,
) -> FoundryPassthroughPlan<NoChatReplyProjection, Stream>
where
    Stream: ChatStreamAssembly,
{
    FoundryPassthroughPlan {
        global_api_base: None,
        global_api_key: None,
        azure: AzurePassthroughConfig {
            api_base_env: None,
            api_key_env: None,
            model_call_details: None,
            chat_reply: NoChatReplyProjection,
            chat_stream,
        },
        ocr: None,
        litellm_params: Object::new(),
    }
}

pub(super) const fn target<'call>(
    model: &'call str,
    endpoint: &'call str,
) -> PassthroughTarget<'call> {
    PassthroughTarget {
        model,
        custom_llm_provider: "azure_ai",
        endpoint,
    }
}

// fixture JSON enters production types only through serde
pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("fixture object")
}

pub(super) fn parameters(text: &str) -> ParamsObject {
    serde_json::from_str(text).expect("fixture params")
}

pub(super) const fn reply<'body>(
    status: u16,
    headers: &'body Headers,
    body: &'body [u8],
) -> ResponseReply<'body> {
    ResponseReply {
        status,
        headers,
        body,
    }
}

pub(super) struct Offline;

fn refused() -> SourceFailure {
    SourceFailure {
        class: String::from("ValueError"),
        status: None,
        message: String::from(
            "the foundry passthrough adapter resolves no Azure credential source",
        ),
    }
}

impl CredentialSources for Offline {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(refused())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(refused())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(refused())
    }

    fn default_credential<'source>(
        &self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(refused())
    }
}

fn environment_plan(base: &str, key: Option<&str>) -> Plan {
    Plan {
        global_api_base: None,
        global_api_key: None,
        azure: AzurePassthroughConfig {
            api_base_env: Some(base.to_owned()),
            api_key_env: key.map(SecretString::from),
            model_call_details: None,
            chat_reply: NoChatReplyProjection,
            chat_stream: NoChatStreamAssembly,
        },
        ocr: None,
        litellm_params: Object::new(),
    }
}

fn identity_headers(plan: &Plan) -> Result<Headers, CredentialsError> {
    let environment = BTreeMap::new();
    let sources = Offline;
    let base = plan.accessors().api_base(None);
    let key = plan.accessors().api_key(None);
    validate_environment(ValidateEnvironmentRequest {
        headers: BTreeMap::new(),
        api_key: key.as_ref(),
        params: None,
        api_base: base.as_deref(),
        environment: &environment,
        refresh_enabled: false,
        sources: &sources,
    })
}

fn assert_identity(plan: &Plan, base: &str, key: &str) {
    assert_eq!(plan.accessors().api_base(None).as_deref(), Some(base));
    assert_eq!(
        plan.accessors()
            .api_key(None)
            .map(|secret| secret.expose_secret().to_owned()),
        Some(key.to_owned())
    );
    let headers = identity_headers(plan).unwrap();
    assert_eq!(headers.get("api-key").map(String::as_str), Some(key));
}

#[test]
fn passthrough_credential_identity_isolation() {
    let first = environment_plan("https://one.services.ai.azure.com", Some("key-one"));
    let second = environment_plan("https://two.services.ai.azure.com", Some("key-two"));
    assert_identity(&first, "https://one.services.ai.azure.com", "key-one");
    assert_identity(&second, "https://two.services.ai.azure.com", "key-two");
    let third = environment_plan("https://three.services.ai.azure.com", None);
    let error = identity_headers(&third).unwrap_err();
    assert!(matches!(error, CredentialsError::MissingCredentials { .. }));
}

fn secret_plan() -> Plan {
    Plan {
        global_api_base: None,
        global_api_key: Some(SecretString::from("fixture-global-secret")),
        azure: AzurePassthroughConfig {
            api_base_env: Some(String::from("https://res.services.ai.azure.com")),
            api_key_env: Some(SecretString::from("fixture-env-secret")),
            model_call_details: None,
            chat_reply: NoChatReplyProjection,
            chat_stream: NoChatStreamAssembly,
        },
        ocr: None,
        litellm_params: Object::new(),
    }
}

fn assert_plan_debug_redacts(plan: &Plan) {
    let debug = format!("{plan:?}");
    assert!(debug.contains("FoundryPassthroughPlan"), "{debug}");
    assert!(debug.contains("<redacted>"), "{debug}");
    for secret in ["fixture-global-secret", "fixture-env-secret"] {
        assert!(!debug.contains(secret), "{debug}");
    }
}

#[test]
fn provider_errors_keep_their_message() {
    let parts = ProviderErrorParts {
        status_code: 400_u16,
        message: String::from("fixture provider failure"),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    };
    let error = FoundryPassthroughError::from(ProviderError::from(parts));
    assert_eq!(error.to_string(), "fixture provider failure");
}

fn offline_entra_error() -> CredentialsError {
    let environment = BTreeMap::from([(
        String::from("AZURE_AI_API_KEY"),
        String::from("fixture-environment-secret"),
    )]);
    let params = parameters(
        r#"{"tenant_id": "fixture-tenant", "client_id": "fixture-client", "client_secret": "fixture-params-secret"}"#,
    );
    let sources = Offline;
    validate_environment(ValidateEnvironmentRequest {
        headers: BTreeMap::new(),
        api_key: Some(&SecretString::from("")),
        params: Some(&params),
        api_base: None,
        environment: &environment,
        refresh_enabled: false,
        sources: &sources,
    })
    .unwrap_err()
}

#[test]
fn passthrough_secret_safe_failure() {
    let plan = secret_plan();
    assert_plan_debug_redacts(&plan);
    let error = offline_entra_error();
    let rendered = format!("{error:?} {error}");
    let leaked = [
        "fixture-environment-secret",
        "fixture-params-secret",
        "fixture-global-secret",
    ]
    .into_iter()
    .any(|secret| rendered.contains(secret));
    assert!(!leaked, "{rendered}");
    let root_error = foundry_root("https://user:fixture-url-secret@host/a\nb").unwrap_err();
    let root_rendered = format!("{root_error:?} {root_error}");
    assert!(
        !root_rendered.contains("fixture-url-secret"),
        "{root_rendered}"
    );
    assert!(matches!(
        root_error,
        FoundryPassthroughError::InvalidApiBase(_)
    ));
}

fn corpus_failure(path: &Path) -> Option<String> {
    let bytes = fs::read(path).expect("corpus file");
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("corpus name");
    let expected_ok = name.starts_with("ok-");
    let outcome = decode_logged_ocr_reply(None, OCR_MODEL, OCR_ENDPOINT, &bytes);
    let matched = match &outcome {
        Ok(Some(_)) => expected_ok,
        Ok(None) => false,
        Err(_) => !expected_ok,
    };
    if matched {
        None
    } else {
        Some(format!("{name}: {outcome:?}"))
    }
}

#[test]
fn replays_the_foundry_passthrough_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/foundry_passthrough");
    let paths: Vec<_> = fs::read_dir(&corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!paths.is_empty());
    let failures: Vec<String> = paths
        .iter()
        .filter_map(|path| corpus_failure(path.as_path()))
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}
