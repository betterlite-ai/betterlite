use std::collections::{BTreeMap, HashMap};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::azure_endpoints::Settings;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::embedding::EmbeddingInput;
use secrecy::SecretString;
use tokio::runtime::Builder;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::embeddings::call::{ClientContext, Dependencies, EmbeddingCall, embedding};
use crate::embeddings::client::{
    ClientFactory, EmbeddingClient, Fields, Headers, RawEmbedding, SdkError,
};
use crate::embeddings::error::EmbeddingError;
use crate::embeddings::logging::{EmbeddingLogger, PostCallRecord, PreCallRecord};
use crate::embeddings::reply::{EmbeddingReply, ModelResponse};
use crate::endpoints::get_client::ClientCache;
use crate::endpoints::projection::{
    ClientIdentity, ClientInputs, ClientKind, ClientOutcome, LiteLlmParams, SessionKind,
};
use crate::oidc_exchange::OidcExchangeInput;

pub(super) const BODY: &str = r#"{"object":"list","data":[{"object":"embedding","embedding":[0.5,0.25],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":3,"total_tokens":3}}"#;

#[derive(Clone)]
pub(super) enum Answer {
    Reply {
        status: u16,
        headers: Headers,
        body: String,
    },
    Failure(SdkError),
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn json_reply(extra: &[(&str, &str)]) -> Answer {
    let mut fields = headers(&[("content-type", "application/json")]);
    fields.extend(headers(extra));
    Answer::Reply {
        status: 200,
        headers: fields,
        body: BODY.to_owned(),
    }
}

pub(super) fn text_reply(status: u16, content_type: &str, body: &str) -> Answer {
    Answer::Reply {
        status,
        headers: headers(&[("content-type", content_type)]),
        body: body.to_owned(),
    }
}

pub(super) type Calls = Arc<Mutex<Vec<(Fields, f64)>>>;

pub(super) struct Sdk {
    pub(super) kind: ClientKind,
    pub(super) answer: Answer,
    pub(super) calls: Calls,
}

impl EmbeddingClient for Sdk {
    fn kind(&self) -> ClientKind {
        self.kind
    }

    fn create(
        &self,
        data: &Fields,
        timeout: f64,
    ) -> impl Future<Output = Result<RawEmbedding, SdkError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((data.clone(), timeout));
        ready(match &self.answer {
            Answer::Reply {
                status,
                headers,
                body,
            } => Ok(RawEmbedding {
                status: *status,
                headers: headers.clone(),
                body: body.clone().into_bytes(),
            }),
            Answer::Failure(error) => Err(error.clone()),
        })
    }
}

// what the constructed client would receive: its kind, endpoint, API version, retries and timeout
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Built {
    pub(super) kind: ClientKind,
    pub(super) endpoint: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) max_retries: Option<i64>,
    pub(super) timeout: Option<f64>,
}

pub(super) struct Factory {
    answer: Answer,
    calls: Calls,
    built: Mutex<Vec<Built>>,
}

impl ClientFactory for Factory {
    type Client = Sdk;

    fn client(&self, outcome: &ClientOutcome<'_>) -> Result<Sdk, SdkError> {
        let mut built = Built {
            kind: outcome.kind,
            endpoint: None,
            api_version: None,
            max_retries: None,
            timeout: None,
        };
        match &outcome.inputs {
            Some(ClientInputs::Azure(inputs)) => {
                built.endpoint.clone_from(&inputs.endpoint.azure_endpoint);
                built.api_version = Some(inputs.api_version.clone());
                built.max_retries = Some(inputs.max_retries);
                built.timeout = inputs.timeout;
            }
            Some(ClientInputs::OpenAiV1(inputs)) => {
                built.endpoint.clone_from(&inputs.base_url);
                built.max_retries = Some(inputs.max_retries);
                built.timeout = inputs.timeout;
            }
            None => {}
        }
        self.built
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(built);
        Ok(Sdk {
            kind: outcome.kind,
            answer: self.answer.clone(),
            calls: Arc::clone(&self.calls),
        })
    }
}

#[derive(Default)]
pub(super) struct Recorder {
    pub(super) pre: Vec<PreCallRecord>,
    pub(super) post: Vec<PostCallRecord>,
    pub(super) fail_pre_call: bool,
}

impl EmbeddingLogger for Recorder {
    fn pre_call(&mut self, record: PreCallRecord) -> Result<(), PrePostError> {
        self.pre.push(record);
        if self.fail_pre_call {
            return Err(PrePostError::CallbackFailed {
                message: "pre-call failed".to_owned(),
            });
        }
        Ok(())
    }

    fn post_call(&mut self, record: PostCallRecord) {
        self.post.push(record);
    }
}

#[derive(Default)]
struct Cache(HashMap<ClientIdentity, ClientKind>);

impl ClientCache for Cache {
    fn get(&self, identity: &ClientIdentity) -> Option<ClientKind> {
        self.0.get(identity).copied()
    }

    fn set(&mut self, identity: ClientIdentity, kind: ClientKind, _owned: bool) {
        self.0.insert(identity, kind);
    }
}

fn no_source() -> SourceFailure {
    SourceFailure {
        class: "RuntimeError".to_owned(),
        status: None,
        message: "no credential source in this fixture".to_owned(),
    }
}

struct NoSources;

type Provider<'source> = Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure>;

impl CredentialSources for NoSources {
    fn client_secret(&self, _record: &ClientSecretRecord) -> Provider<'_> {
        Err(no_source())
    }

    fn username_password(&self, _record: &UsernamePasswordRecord) -> Provider<'_> {
        Err(no_source())
    }

    fn sdk_refresh(&self, _scope: &str) -> Provider<'_> {
        Err(no_source())
    }

    fn default_credential(&self, _scope: &str) -> Option<Arc<dyn CallableTokenProvider + '_>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(no_source())
    }
}

pub(super) struct Run {
    pub(super) result: Result<Option<EmbeddingReply>, EmbeddingError>,
    pub(super) recorder: Recorder,
    pub(super) calls: Vec<(Fields, f64)>,
    pub(super) built: Vec<Built>,
}

pub(super) fn call<'call>(
    input: &'call EmbeddingInput,
    aembedding: bool,
) -> EmbeddingCall<'call, Sdk> {
    EmbeddingCall {
        model: "my-deployment",
        input,
        api_base: "https://example.openai.azure.com",
        api_version: Some("2024-02-01"),
        timeout: 30.0,
        optional_params: Fields::new(),
        api_key: Some(SecretString::from("azure-key")),
        azure_ad_token: None,
        max_retries: None,
        client: None,
        aembedding,
        headers: Headers::new(),
        litellm_params: LiteLlmParams::default(),
        model_response: ModelResponse::Embedding,
    }
}

pub(super) fn supplied(kind: ClientKind, answer: &Answer, calls: &Calls) -> Sdk {
    Sdk {
        kind,
        answer: answer.clone(),
        calls: Arc::clone(calls),
    }
}

pub(super) fn run(
    request: EmbeddingCall<'_, Sdk>,
    answer: &Answer,
    calls: &Calls,
    recorder: Recorder,
) -> Run {
    let factory = Factory {
        answer: answer.clone(),
        calls: Arc::clone(calls),
        built: Mutex::new(Vec::new()),
    };
    let settings = Settings::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let sources = NoSources;
    let mut cache = Cache::default();
    let mut refresh_cache = RefreshProviderCache::new(&sources);
    let mut logger = recorder;
    let result = {
        let mut dependencies = Dependencies {
            factory: &factory,
            context: ClientContext {
                provider: None,
                settings: &settings,
                environment: &environment,
                refresh_enabled: false,
                session: SessionKind::Dedicated,
                sources: &sources,
            },
            cache: &mut cache,
            refresh_cache: &mut refresh_cache,
            logger: &mut logger,
        };
        Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(embedding(request, &mut dependencies))
    };
    let seen = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let built = factory
        .built
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Run {
        result,
        recorder: logger,
        calls: seen,
        built,
    }
}
