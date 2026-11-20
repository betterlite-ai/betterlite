use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::types::Config;
use betterlite_context::resource_config_selection::responses::ResponsesModelFacts;
use betterlite_core::api::responses_lifecycle_call::error::LifecycleFailure;
use betterlite_core::api::responses_lifecycle_call::executor::HttpReadExecutor;
use betterlite_core::api::responses_lifecycle_call::input::ReadIdentity;
use betterlite_core::api::responses_lifecycle_call::service::{LifecycleLog, ResponsesLifecycle};
use betterlite_core::api::responses_read_http::config::{
    NoWorkloadIdentity, OpenAiReadConfig, ReadEnvironment,
};
use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::response_resources::response::TypedReply;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};

use super::super::error::SdkHttpError;
use super::super::result::Failure;
use super::super::result::kinds::SdkResult;
use super::support::{Fixture, Source, pre_post, source};
use super::{Call, Outcome, Retrieved, Step, StepOutcome};

type Lifecycle = ResponsesLifecycle<HttpReadExecutor<OpenAiReadConfig<NoWorkloadIdentity>, Source>>;

// The production lifecycle service over the mock's base: the same config sections and pooled client a request builds
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let config = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let service = build_service(&config, env)?;
    let mut steps = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        steps.push(run_step(step, &service, api_base).await?);
    }
    Ok(SdkResult::ResponsesLifecycleRead {
        value: Box::new(Outcome { steps }),
    })
}

fn build_service(
    config: &Config,
    env: &BTreeMap<String, String>,
) -> Result<Lifecycle, SdkHttpError> {
    let settings = &config.sections.resource_sdk_settings;
    let provider = OpenAiReadConfig::new(
        LiteLlmGlobals::default(),
        ReadEnvironment {
            api_key: settings.openai_api_key.clone(),
            base_url: settings.openai_base_url.clone(),
            api_base: settings.openai_api_base.clone(),
        },
        NoWorkloadIdentity,
    );
    Ok(ResponsesLifecycle {
        executor: HttpReadExecutor {
            config: provider,
            source: source(config, env)?,
        },
        default_timeout: default_timeout(config)?,
        facts: ResponsesModelFacts::default(),
        redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?,
    })
}

fn default_timeout(config: &Config) -> Result<Duration, SdkHttpError> {
    Duration::try_from_secs_f64(config.sections.routing_timeouts.global.seconds)
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))
}

async fn run_step(
    step: &Step,
    service: &Lifecycle,
    api_base: &str,
) -> Result<StepOutcome, SdkHttpError> {
    let mut fixture = Fixture::new()?;
    let env = pre_post(&service.redactor, &fixture.sink, fixture.now);
    let log = LifecycleLog {
        state: &mut fixture.state,
        env,
    };
    match step {
        Step::GetResponses(inner) => {
            let mut call = inner.call.clone();
            fill_base(&mut call.identity, api_base);
            Ok(reply_outcome(service.get_responses(&call, log).await))
        }
        Step::AgetResponses(inner) => {
            let mut call = inner.call.clone();
            fill_base(&mut call.identity, api_base);
            Ok(reply_outcome(service.aget_responses(&call, log).await))
        }
        Step::ListInputItems(inner) => {
            let mut call = inner.call.clone();
            fill_base(&mut call.identity, api_base);
            Ok(items_outcome(service.list_input_items(&call, log).await))
        }
        Step::AlistInputItems(inner) => {
            let mut call = inner.call.clone();
            fill_base(&mut call.identity, api_base);
            Ok(items_outcome(service.alist_input_items(call, log).await))
        }
    }
}

// the SDK reads a null base as its own default, so the harness base stands in for an absent one
fn fill_base(identity: &mut ReadIdentity, api_base: &str) {
    if identity.api_base.is_none() {
        identity.api_base = Some(api_base.to_owned());
    }
}

fn reply_outcome(reply: Result<TypedReply, Box<LifecycleFailure>>) -> StepOutcome {
    match reply {
        Ok(reply) => success(Some(retrieved(&reply)), None),
        Err(failure) => failed(*failure),
    }
}

fn items_outcome(items: Result<Json, Box<LifecycleFailure>>) -> StepOutcome {
    match items {
        Ok(items) => success(None, Some(items)),
        Err(failure) => failed(*failure),
    }
}

fn success(retrieved: Option<Retrieved>, items: Option<Json>) -> StepOutcome {
    StepOutcome {
        retrieved,
        items,
        failure: None,
    }
}

fn failed(failure: LifecycleFailure) -> StepOutcome {
    StepOutcome {
        retrieved: None,
        items: None,
        failure: Some(failure_of(failure)),
    }
}

fn retrieved(reply: &TypedReply) -> Retrieved {
    Retrieved {
        id: reply.response.id.clone(),
        created_at: reply.response.created_at,
        status: reply.response.status.clone(),
        model: reply.response.model.clone(),
    }
}

// The exception's class, status and flat fields, which Python's APIError reports as null rather than absent
fn failure_of(failure: LifecycleFailure) -> Failure {
    Failure {
        class: failure.class.to_owned(),
        status: failure.status,
        kind: Some(optional_text(&failure.kind)),
        param: Some(optional_text(&failure.param)),
        code: Some(optional_text(&failure.code)),
        message: failure.message,
    }
}

fn optional_text(wire: &WireField) -> Option<String> {
    match wire {
        WireField::Missing(_) | WireField::Null => None,
        WireField::Text(text) => Some(text.clone()),
    }
}
