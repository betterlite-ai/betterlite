use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;

use betterlite_core::api::speech_call_plan::{
    AspeechKeywords, AsyncSpeech, SpeechInvoker, SpeechReturn,
};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, resolve,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use serde::{Deserialize, Serialize};

use super::OracleLimits;
use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;
use super::speech_call::{self, API_BASE_SENTINEL, BinaryValue, Globals, StepOutcome};

use projection::project;

mod projection;

// one aspeech() call; a step without speech arguments passes aspeech no model at all
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    #[serde(default)]
    pub speech: Option<speech_call::Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    #[serde(default)]
    pub globals: Globals,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub steps: Vec<StepOutcome>,
}

// how one speech() invocation fails here: B1084's projected failure, the wrapper's resolver, or the harness
#[derive(Debug)]
enum Raised {
    Speech(Failure),
    Resolution(ResolutionError),
    Harness(SdkHttpError),
}

impl From<ResolutionError> for Raised {
    fn from(source: ResolutionError) -> Self {
        Self::Resolution(source)
    }
}

impl AspeechKeywords for Step {
    fn model(&self) -> Option<&str> {
        self.speech.as_ref().map(|speech| speech.model.as_str())
    }

    fn api_base(&self) -> Option<&str> {
        self.speech
            .as_ref()
            .and_then(|speech| speech.api_base.as_deref())
    }

    fn custom_llm_provider(&self) -> Option<&str> {
        self.speech
            .as_ref()
            .and_then(|speech| speech.custom_llm_provider.as_deref())
    }

    // speech() takes the flag as its own argument, so it never stays in the caller's kwargs
    fn force_async(&mut self) {
        if let Some(speech) = &mut self.speech {
            speech.kwargs.remove("aspeech");
        }
    }
}

type CasePending<'case> = Pin<Box<dyn Future<Output = Result<BinaryValue, Raised>> + Send + 'case>>;

// speech() for one case step: B1084's native speech_call against the mock, returned the way speech() returns it
struct CaseSpeech<'case> {
    globals: &'case Globals,
    api_base: &'case str,
    env: &'case BTreeMap<String, String>,
    resolution: &'case ResolutionContext<'case>,
}

impl<'case> SpeechInvoker<Step> for CaseSpeech<'case> {
    type Context = ();
    type Answer = BinaryValue;
    type Error = Raised;
    type Pending = CasePending<'case>;

    fn speech(
        &self,
        keywords: &Step,
        _context: &(),
    ) -> impl Future<Output = Result<SpeechReturn<BinaryValue, CasePending<'case>>, Raised>> + Send
    {
        let finished = keywords
            .speech
            .as_ref()
            .is_some_and(|step| bridged(step, self.resolution));
        let single = speech_call::Call {
            globals: self.globals.clone(),
            steps: keywords.speech.iter().cloned().collect(),
        };
        let (api_base, env) = (self.api_base, self.env);
        let pending: CasePending<'case> =
            Box::pin(async move { answer(single.send(api_base, env).await) });
        async move {
            if finished {
                pending.await.map(SpeechReturn::Finished)
            } else {
                Ok(SpeechReturn::Coroutine(pending))
            }
        }
    }
}

// speech() runs the chat bridge to its end even with aspeech=True, so gemini TTS hands back no coroutine
fn bridged(step: &speech_call::Step, resolution: &ResolutionContext<'_>) -> bool {
    let request = ResolutionRequest {
        model: &step.model,
        custom_llm_provider: step.custom_llm_provider.as_deref(),
        api_base: step.api_base.as_deref().or(step.base_url.as_deref()),
        api_key: None,
    };
    resolve(&request, resolution).is_ok_and(|resolved| match resolved.provider.slug() {
        "gemini" => true,
        "vertex_ai" | "vertex_ai_beta" => resolved.model.contains("gemini"),
        _ => false,
    })
}

fn answer(result: Result<SdkResult, SdkHttpError>) -> Result<BinaryValue, Raised> {
    let unexpected = || {
        Raised::Harness(SdkHttpError::Invalid(String::from(
            "speech_call answered without exactly one step outcome",
        )))
    };
    let SdkResult::SpeechCall { value } = result.map_err(Raised::Harness)? else {
        return Err(unexpected());
    };
    let mut steps = value.steps.into_iter();
    let first = steps.next();
    if steps.next().is_some() {
        return Err(unexpected());
    }
    match first {
        Some(StepOutcome {
            value: Some(audio),
            failure: None,
        }) => Ok(audio),
        Some(StepOutcome {
            value: None,
            failure: Some(failure),
        }) => Err(Raised::Speech(failure)),
        Some(
            StepOutcome {
                value: Some(_),
                failure: Some(_),
            }
            | StepOutcome {
                value: None,
                failure: None,
            },
        )
        | None => Err(unexpected()),
    }
}

impl CallKind for Call {
    fn validate(&self, limits: &OracleLimits) -> Result<(), SdkHttpError> {
        if self.steps.is_empty() {
            return Err(SdkHttpError::Invalid(String::from(
                "a speech_call_plan call needs at least one step",
            )));
        }
        for speech in self.steps.iter().filter_map(|step| step.speech.clone()) {
            speech_call::Call {
                globals: self.globals.clone(),
                steps: vec![speech],
            }
            .validate(limits)?;
        }
        Ok(())
    }

    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        execute(self, api_base, env)
    }
}

async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let environment: BTreeMap<String, String> = env
        .iter()
        .map(|(name, value)| {
            let resolved = if value == API_BASE_SENTINEL {
                api_base
            } else {
                value.as_str()
            };
            (name.clone(), resolved.to_owned())
        })
        .collect();
    let catalog = ModelCatalog::default();
    let names = BTreeSet::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &names,
        custom_providers: &names,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let invoker = CaseSpeech {
        globals: &call.globals,
        api_base,
        env,
        resolution: &resolution,
    };
    let service = AsyncSpeech {
        invoker: &invoker,
        resolution: &resolution,
    };
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        let result = service.aspeech(step.clone(), &()).await;
        steps.push(project(result, &redactor)?);
    }
    Ok(SdkResult::SpeechCallPlan {
        value: Box::new(Outcome { steps }),
    })
}
