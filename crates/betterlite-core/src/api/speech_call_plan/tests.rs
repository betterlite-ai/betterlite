#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::task::{Context, Poll};

use betterlite_error::error::ErrorClass;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use rstest::rstest;

use crate::api::speech_call::call::SpeechCall;
use crate::api::speech_call::error::{SpeechCallError, SpeechFailure};

use super::error::AspeechError;
use super::{AspeechKeywords, AsyncSpeech, SpeechInvoker, SpeechReturn};

const MARKER: &str = "request-7";
const NATIVE: &str = "openai/tts-1";
const GEMINI: &str = "gemini/gemini-2.5-flash-preview-tts";
const GROQ_BASE: &str = "https://api.groq.com/openai/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Audio {
    Bytes(Vec<u8>),
    // a value of a type speech() never returns, standing in for any unexpected second result
    Unexpected(&'static str),
}

// what one scripted speech() invocation hands back
enum Scripted {
    Coroutine(Result<Audio, SpeechCallError>),
    Finished(Audio),
    Unawaited,
    Raise(SpeechCallError),
}

// a coroutine that resolves once, or one the wrapper must hand back without polling it
#[derive(Debug)]
enum Pending {
    Resolves(Option<Result<Audio, SpeechCallError>>),
    Unawaited,
}

impl Future for Pending {
    type Output = Result<Audio, SpeechCallError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::Resolves(result) => {
                Poll::Ready(result.take().expect("aspeech awaits a coroutine once"))
            }
            Self::Unawaited => panic!("aspeech awaited a second result it must return unchecked"),
        }
    }
}

// aspeech's kwargs: a speech() call, or a call that named no model at all
struct Keywords(Option<SpeechCall>);

impl AspeechKeywords for Keywords {
    fn model(&self) -> Option<&str> {
        self.0.as_ref().map(|call| call.model.as_str())
    }

    fn api_base(&self) -> Option<&str> {
        self.0.as_ref()?.api_base.as_deref()
    }

    fn custom_llm_provider(&self) -> Option<&str> {
        self.0.as_ref()?.custom_llm_provider.as_deref()
    }

    fn force_async(&mut self) {
        if let Some(call) = &mut self.0 {
            call.aspeech = true;
        }
    }
}

// what speech() saw: the flag, the caller's provider and the request context's marker
#[derive(Debug, Clone, PartialEq, Eq)]
struct Invocation {
    model: String,
    aspeech: bool,
    custom_llm_provider: Option<String>,
    marker: &'static str,
}

struct RequestContext {
    marker: &'static str,
}

struct ScriptedSpeech {
    script: Mutex<VecDeque<Scripted>>,
    invocations: Mutex<Vec<Invocation>>,
}

impl SpeechInvoker<Keywords> for ScriptedSpeech {
    type Context = RequestContext;
    type Answer = Audio;
    type Error = SpeechCallError;
    type Pending = Pending;

    fn speech(
        &self,
        keywords: &Keywords,
        context: &RequestContext,
    ) -> impl Future<Output = Result<SpeechReturn<Audio, Pending>, SpeechCallError>> + Send {
        let call = keywords
            .0
            .as_ref()
            .expect("speech() runs only with a model");
        self.invocations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Invocation {
                model: call.model.clone(),
                aspeech: call.aspeech,
                custom_llm_provider: call.custom_llm_provider.clone(),
                marker: context.marker,
            });
        let next = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("a scripted speech() answer");
        ready(match next {
            Scripted::Coroutine(result) => {
                Ok(SpeechReturn::Coroutine(Pending::Resolves(Some(result))))
            }
            Scripted::Finished(audio) => Ok(SpeechReturn::Finished(audio)),
            Scripted::Unawaited => Ok(SpeechReturn::Coroutine(Pending::Unawaited)),
            Scripted::Raise(error) => Err(error),
        })
    }
}

struct Run {
    result: Result<SpeechReturn<Audio, Pending>, AspeechError<SpeechCallError>>,
    invocations: Vec<Invocation>,
}

async fn run(keywords: Keywords, script: Vec<Scripted>) -> Run {
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let invoker = ScriptedSpeech {
        script: Mutex::new(VecDeque::from(script)),
        invocations: Mutex::new(Vec::new()),
    };
    let service = AsyncSpeech {
        invoker: &invoker,
        resolution: &resolution,
    };
    let result = service
        .aspeech(keywords, &RequestContext { marker: MARKER })
        .await;
    Run {
        result,
        invocations: invoker
            .invocations
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner),
    }
}

fn call(model: &str) -> Keywords {
    Keywords(Some(SpeechCall {
        model: model.to_owned(),
        input: String::from("hello"),
        voice: Some(SpeechVoice::Name(String::from("alloy"))),
        api_key: None,
        api_base: None,
        api_version: None,
        organization: None,
        project: None,
        custom_llm_provider: None,
        base_url: None,
        max_retries: Some(0),
        timeout_seconds: None,
        metadata: None,
        response_format: None,
        speed: None,
        instructions: None,
        headers: None,
        client: None,
        shared_session: None,
        aspeech: false,
        kwargs: Object::new(),
    }))
}

fn supplied(model: &str, change: fn(&mut SpeechCall)) -> Keywords {
    let mut keywords = call(model);
    if let Some(speech) = &mut keywords.0 {
        change(speech);
    }
    keywords
}

fn invocation(model: &str, custom_llm_provider: Option<&str>) -> Invocation {
    Invocation {
        model: model.to_owned(),
        aspeech: true,
        custom_llm_provider: custom_llm_provider.map(str::to_owned),
        marker: MARKER,
    }
}

fn rate_limited() -> SpeechCallError {
    SpeechCallError::Provider {
        status: 429,
        message: String::from("slow down"),
        error_type: Some(String::from("rate_limit_error")),
        param: Some(String::from("model")),
        code: Some(String::from("rate_limit")),
    }
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("a redactor")
}

fn raised(run: &Run) -> (&str, &str, &SpeechCallError) {
    let Err(AspeechError::Raised {
        model,
        provider,
        error,
    }) = &run.result
    else {
        panic!("the failure is raised inside aspeech's try");
    };
    (model.as_str(), provider.as_str(), error)
}

#[tokio::test]
async fn async_native_speech() {
    let script = vec![Scripted::Coroutine(Ok(Audio::Bytes(vec![0_u8, 1, 2])))];
    let outcome = run(call(NATIVE), script).await;
    let Ok(SpeechReturn::Finished(audio)) = outcome.result else {
        panic!("the awaited coroutine's audio comes back");
    };
    assert_eq!(audio, Audio::Bytes(vec![0_u8, 1, 2]));
    assert_eq!(outcome.invocations, vec![invocation(NATIVE, None)]);
}

#[tokio::test]
async fn async_bridged_speech_calls_twice() {
    let script = vec![
        Scripted::Finished(Audio::Bytes(vec![0_u8, 0])),
        Scripted::Finished(Audio::Bytes(vec![1_u8, 1])),
    ];
    let outcome = run(call(GEMINI), script).await;
    let Ok(SpeechReturn::Finished(audio)) = outcome.result else {
        panic!("the second call's audio comes back");
    };
    assert_eq!(audio, Audio::Bytes(vec![1_u8, 1]));
    assert_eq!(outcome.invocations, vec![invocation(GEMINI, None); 2]);
}

#[tokio::test]
async fn a_native_immediate_response_returns_an_unexpected_second_result_unchecked() {
    let script = vec![
        Scripted::Finished(Audio::Bytes(vec![0_u8, 1, 2])),
        Scripted::Finished(Audio::Unexpected("dict")),
    ];
    let outcome = run(call(NATIVE), script).await;
    let Ok(SpeechReturn::Finished(audio)) = outcome.result else {
        panic!("the second call's value comes back");
    };
    assert_eq!(audio, Audio::Unexpected("dict"));
    assert_eq!(outcome.invocations, vec![invocation(NATIVE, None); 2]);
}

#[tokio::test]
async fn a_second_coroutine_comes_back_unawaited() {
    let script = vec![
        Scripted::Finished(Audio::Bytes(vec![0_u8, 1, 2])),
        Scripted::Unawaited,
    ];
    let outcome = run(call(NATIVE), script).await;
    assert!(matches!(
        outcome.result,
        Ok(SpeechReturn::Coroutine(Pending::Unawaited))
    ));
    assert_eq!(outcome.invocations.len(), 2);
}

#[tokio::test]
async fn async_speech_error_attribution() {
    let outcome = run(Keywords(None), Vec::new()).await;
    let Err(error) = &outcome.result else {
        panic!("a call without a model fails");
    };
    assert!(matches!(error, AspeechError::MissingModel));
    assert_eq!(error.to_string(), "'model'");
    assert_eq!(error.exception(&redactor()), None);
    assert!(outcome.invocations.is_empty());
}

#[rstest]
#[case::provider_unset(None, "openai")]
#[case::provider_supplied(Some("azure"), "azure")]
#[tokio::test]
async fn a_resolver_failure_maps_under_the_caller_provider(
    #[case] provider: Option<&'static str>,
    #[case] expected: &str,
) {
    let mut unknown = call("not-a-model");
    if let Some(speech) = &mut unknown.0 {
        speech.custom_llm_provider = provider.map(str::to_owned);
    }
    let outcome = run(unknown, vec![Scripted::Raise(rate_limited())]).await;
    let (model, attributed, error) = raised(&outcome);
    assert_eq!((model, attributed), ("not-a-model", expected));
    assert!(matches!(error, SpeechCallError::Resolution(_)));
    let mapped = outcome
        .result
        .as_ref()
        .err()
        .and_then(|failure| failure.exception(&redactor()))
        .map(|exception| exception.failure);
    assert!(matches!(
        mapped,
        Some(SpeechFailure::Mapped(ErrorClass::BadRequest(_)))
    ));
    assert!(outcome.invocations.is_empty());
}

#[tokio::test]
async fn a_supplied_provider_is_replaced_by_the_wrapper_resolver() {
    let groq = supplied("groq/playai-tts", |speech| {
        speech.custom_llm_provider = Some(String::from("openai"));
    });
    let outcome = run(groq, vec![Scripted::Raise(rate_limited())]).await;
    let (model, provider, error) = raised(&outcome);
    assert_eq!((model, provider), ("groq/playai-tts", "groq"));
    assert!(matches!(
        error,
        SpeechCallError::Provider { status: 429, .. }
    ));
    let mapped = outcome
        .result
        .as_ref()
        .err()
        .and_then(|failure| failure.exception(&redactor()))
        .map(|exception| exception.failure);
    assert!(matches!(
        mapped,
        Some(SpeechFailure::Mapped(ErrorClass::RateLimit(_)))
    ));
    assert_eq!(
        outcome.invocations,
        vec![invocation("groq/playai-tts", Some("openai"))]
    );
}

#[tokio::test]
async fn a_raising_coroutine_is_not_invoked_again() {
    let script = vec![Scripted::Coroutine(Err(rate_limited()))];
    let outcome = run(call(NATIVE), script).await;
    let (model, provider, error) = raised(&outcome);
    assert_eq!((model, provider), (NATIVE, "openai"));
    assert!(matches!(
        error,
        SpeechCallError::Provider { status: 429, .. }
    ));
    assert_eq!(outcome.invocations, vec![invocation(NATIVE, None)]);
}

#[rstest]
#[case::unmapped_provider(
    "anthropic/claude-3-5-haiku",
    SpeechCallError::Unmapped {
        provider: String::from("anthropic"),
        message: String::from(
            "Unable to map the custom llm provider=anthropic to a known provider=[]."
        ),
    },
    "anthropic"
)]
#[case::transport_failure(
    NATIVE,
    SpeechCallError::Sdk {
        class: "APIConnectionError",
        message: String::from("Connection error."),
    },
    "openai"
)]
#[tokio::test]
async fn a_status_less_failure_maps_as_an_api_connection(
    #[case] model: &'static str,
    #[case] failure: SpeechCallError,
    #[case] provider: &'static str,
) {
    let outcome = run(call(model), vec![Scripted::Raise(failure)]).await;
    let (raised_model, attributed, error) = raised(&outcome);
    assert_eq!((raised_model, attributed), (model, provider));
    assert!(matches!(
        error,
        SpeechCallError::Unmapped { .. } | SpeechCallError::Sdk { .. }
    ));
    let exception = outcome
        .result
        .as_ref()
        .err()
        .and_then(|thrown| thrown.exception(&redactor()))
        .expect("a failure inside the try has a litellm class");
    let SpeechFailure::Mapped(class @ ErrorClass::ApiConnection(_)) = &exception.failure else {
        panic!("a status-less failure becomes an APIConnectionError");
    };
    assert_eq!(class.metadata().status_code, 500_u16);
    assert_eq!(exception.message, error.to_string());
}

#[tokio::test]
async fn a_timed_out_transport_maps_as_a_timeout() {
    let timed_out = SpeechCallError::Sdk {
        class: "APITimeoutError",
        message: String::from("Request timed out."),
    };
    let outcome = run(call(NATIVE), vec![Scripted::Raise(timed_out)]).await;
    let exception = outcome
        .result
        .as_ref()
        .err()
        .and_then(|thrown| thrown.exception(&redactor()))
        .expect("a failure inside the try has a litellm class");
    let SpeechFailure::Mapped(class @ ErrorClass::Timeout(_)) = &exception.failure else {
        panic!("the SDK's timeout text reaches exception_type's timeout preamble");
    };
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 408_u16);
    assert_eq!(metadata.provider.as_deref(), Some("openai"));
    assert_eq!(outcome.invocations, vec![invocation(NATIVE, None)]);
}

#[rstest]
#[case::api_base_names_the_provider(
    supplied("tts-1", |speech| speech.api_base = Some(String::from(GROQ_BASE))),
    "groq",
    1
)]
#[case::base_url_is_not_read(
    supplied("tts-1", |speech| speech.base_url = Some(String::from(GROQ_BASE))),
    "openai",
    0
)]
#[tokio::test]
async fn the_wrapper_resolver_reads_only_api_base(
    #[case] keywords: Keywords,
    #[case] expected: &str,
    #[case] invocations: usize,
) {
    let outcome = run(keywords, vec![Scripted::Raise(rate_limited())]).await;
    let (_, provider, _) = raised(&outcome);
    assert_eq!(provider, expected);
    assert_eq!(outcome.invocations.len(), invocations);
}
