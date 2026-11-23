use super::super::{error::SdkHttpError, result::kinds::SdkResult};
use super::{Call, MOCK, Outcome, Run, RunOutcome, report, support};
use betterlite_core::api::decisions_send::error::DecisionsSendError;
use betterlite_core::api::decisions_send::format::DecisionsOutput;
use betterlite_core::api::decisions_send::send::DecisionsSender;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Extra;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::call::azure::AzureAiConfig;
use betterlite_openai::decisions::call::error::PrepareError;
use betterlite_openai::decisions::call::fields::{
    OpenAiDecisionRequestFields, RequestFields, SystemOneRequestFields,
};
use betterlite_openai::decisions::call::{DecisionsCall, PrepareCall, prepare_call};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::SecretString;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use std::{panic::resume_unwind, thread::scope};

// The case's runs in order, each through the wrapper the case asked for
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let mut runs = Vec::with_capacity(call.runs.len());
    for run in &call.runs {
        let outcome = if run.asynchronous {
            answered(run, api_base, env).await?
        } else {
            blocked(run, api_base, env)?
        };
        runs.push(outcome);
    }
    Ok(SdkResult::DecisionsSend {
        value: Box::new(Outcome { runs }),
    })
}

// decisions() blocks on a client that cannot be built inside a runtime, so it runs on a thread of its own
fn blocked(
    run: &Run,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<RunOutcome, SdkHttpError> {
    scope(|threads| {
        let signed = threads.spawn(|| {
            let mut clients = support::clients(env)?;
            let redactor =
                Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
            let log = support::Sink;
            let log_env = support::prepost_env(&redactor, &log);
            let mut sender = DecisionsSender::new(&mut clients, &log_env);
            let decisions = match prepared_call(run, api_base, env) {
                Ok(decisions) => decisions,
                Err(error) => return report::prepared(&error),
            };
            finished(sender.send(&decisions))
        });
        match signed.join() {
            Ok(outcome) => outcome.map_err(|error| SdkHttpError::Invalid(error.to_string())),
            Err(payload) => resume_unwind(payload),
        }
    })
}

async fn answered(
    run: &Run,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<RunOutcome, SdkHttpError> {
    let mut clients = support::clients(env)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let log = support::Sink;
    let log_env = support::prepost_env(&redactor, &log);
    let mut sender = DecisionsSender::new(&mut clients, &log_env);
    let decisions = match prepared_call(run, api_base, env) {
        Ok(decisions) => decisions,
        Err(error) => return report::prepared(&error),
    };
    finished(sender.asend(&decisions).await)
}

// The arguments a case names, owned and typed before prepare_call borrows them
struct Arguments {
    fields: RequestFields,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    timeout: Option<Duration>,
    extra_headers: Option<BTreeMap<String, String>>,
    kwargs: Extra,
    globals: LiteLlmGlobals,
    azure_ai: AzureAiConfig,
}

// The wrapper's declared arguments, handed to B3513's prepare_call with no logging object and no resolvers
fn prepared_call(
    run: &Run,
    mock_base: &str,
    environment: &BTreeMap<String, String>,
) -> Result<DecisionsCall<'static>, PrepareError> {
    let arguments = arguments_of(run, mock_base)?;
    let catalog = ModelCatalog::default();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &BTreeSet::new(),
        custom_providers: &BTreeSet::new(),
        environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    prepare_call(PrepareCall {
        model: &run.model,
        fields: arguments.fields,
        safety_identifier: run.safety_identifier.as_deref(),
        api_key: arguments.api_key.as_ref(),
        api_base: arguments.api_base.as_deref(),
        timeout: arguments.timeout,
        custom_llm_provider: run.custom_llm_provider.as_deref(),
        extra_headers: arguments.extra_headers.as_ref(),
        kwargs: arguments.kwargs,
        global_drop_params: false,
        globals: arguments.globals,
        azure_ai: arguments.azure_ai,
        resolution: &resolution,
        logging_obj: None,
    })
}

fn arguments_of(run: &Run, mock_base: &str) -> Result<Arguments, PrepareError> {
    let mut kwargs = substituted_kwargs(run, mock_base)?;
    let fields = fields_of(run, &kwargs)?;
    // _prepare_decisions_call forwards every kwarg except state, which only picked the request format
    kwargs.shift_remove("state");
    Ok(Arguments {
        fields,
        api_key: run.api_key.clone().map(SecretString::from),
        api_base: run
            .api_base
            .as_ref()
            .map(|base| base.replace(MOCK, mock_base)),
        timeout: run
            .timeout
            .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok()),
        extra_headers: run.extra_headers.clone(),
        kwargs,
        globals: LiteLlmGlobals::default(),
        azure_ai: AzureAiConfig {
            complete_url: Box::new(|base: &str, _model: &str| base.to_owned()),
        },
    })
}

// The harness names its own origin wherever a case's kwargs carry {mock}, so kwargs substitute through their JSON text
fn substituted_kwargs(run: &Run, mock_base: &str) -> Result<Extra, PrepareError> {
    let text = serde_json::to_string(&run.kwargs)
        .map_err(|source| invalid_request(run, source))?
        .replace(MOCK, mock_base);
    serde_json::from_str(&text).map_err(|source| invalid_request(run, source))
}

// _request_fields: a state key selects the SystemOne envelope, and is filtered out of the kwargs after
fn fields_of(run: &Run, kwargs: &Extra) -> Result<RequestFields, PrepareError> {
    let state = kwargs.get("state").cloned();
    let raw_questions = run.questions.clone();
    if state.is_some() {
        let questions = raw_questions
            .map(|value| serde_json::from_value(serde_json::to_value(value)?))
            .transpose()
            .map_err(|source| invalid_request(run, source))?;
        return Ok(RequestFields::SystemOne(SystemOneRequestFields {
            state,
            questions,
        }));
    }
    let questions = raw_questions
        .map(|value| serde_json::from_value(serde_json::to_value(value)?))
        .transpose()
        .map_err(|source| invalid_request(run, source))?;
    Ok(RequestFields::OpenAi(OpenAiDecisionRequestFields {
        input: run.input.clone(),
        questions,
    }))
}

// prepare_call raises litellm.BadRequestError for an envelope the request TypeAdapter rejects
fn invalid_request(run: &Run, source: serde_json::Error) -> PrepareError {
    PrepareError::InvalidRequest {
        model: run.model.clone(),
        provider: run.custom_llm_provider.clone().unwrap_or_default(),
        source,
    }
}

fn finished(
    result: Result<DecisionsOutput, DecisionsSendError>,
) -> Result<RunOutcome, SdkHttpError> {
    match result {
        Ok(output) => report::output(output),
        Err(DecisionsSendError::Mapped { class, message, .. }) => report::mapped(&class, message),
        Err(DecisionsSendError::Format(error)) => Err(SdkHttpError::Invalid(error.to_string())),
    }
}
