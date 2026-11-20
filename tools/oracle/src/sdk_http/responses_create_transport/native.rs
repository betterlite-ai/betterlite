use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::responses_create_http::config::CreateParams;
use betterlite_core::api::responses_create_http::error::CreateError;
use betterlite_core::api::responses_create_http::service::{
    CreateCall, CreateDeps, CreateHooks, CreateOutcome, StreamSetup, run,
};
use betterlite_core::api::responses_hook_policy::callbacks::{
    CallbackLists, CallbackValue, PolicyCallback,
};
use betterlite_models::decisions::Json;
use secrecy::SecretString;
use serde_json::value::{RawValue, to_raw_value};

use super::super::error::SdkHttpError;
use super::super::responses_compact_transport::Failure;
use super::super::result::kinds::SdkResult;
use super::config::OpenAiCreateConfig;
use super::hooks::{Dispatcher, NoContinuation, NoResolver, Scripted};
use super::support::{Fixture, source};
use super::{Call, Outcome, Setup, Stream, StreamKind};

const PROVIDER: &str = "openai";

// The OpenAI responses config over the case env's key, as both create handlers receive it
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let loaded = ConfigLoader {
        lookup: env.clone(),
    }
    .load("model_list: []")
    .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
    let origin = source(&loaded, env)?;
    let config = OpenAiCreateConfig {
        api_key: env.get("OPENAI_API_KEY").cloned().map(SecretString::from),
    };
    let params = create_params(call, api_base)?;
    let harness = Harness::new(call)?;
    let deps = CreateDeps {
        config: &config,
        source: &origin,
        supplied: None,
    };
    let mut fixture = Fixture::new("responses")?;
    let answer = fixture
        .create(
            &deps,
            call.client_kind(),
            &create_call(call, &params),
            &harness.view(),
        )
        .await;
    Ok(SdkResult::ResponsesCreateTransport {
        value: Box::new(outcome(answer)?),
    })
}

// The callbacks, dispatcher and executor one case runs with, owned so the hooks can borrow them
struct Harness {
    registered: Vec<CallbackValue>,
    dispatcher: Dispatcher,
    resolver: NoResolver,
    executor: NoContinuation,
}

impl Harness {
    fn new(call: &Call) -> Result<Self, SdkHttpError> {
        Ok(Self {
            registered: callbacks(call)?,
            dispatcher: Dispatcher {
                patch: call.pre_hook.clone(),
            },
            resolver: NoResolver,
            executor: NoContinuation,
        })
    }

    fn view(&self) -> CreateHooks<'_, Dispatcher, NoContinuation> {
        CreateHooks {
            callbacks: CallbackLists {
                global: &self.registered,
                dynamic: &[],
            },
            resolver: &self.resolver,
            dispatcher: &self.dispatcher,
            executor: &self.executor,
            logging: None,
        }
    }
}

// dict(litellm_params) as both handlers receive it, with the mock's base under the harness's key
fn create_params(call: &Call, api_base: &str) -> Result<CreateParams, SdkHttpError> {
    let mut map = call.litellm_params.clone();
    map.insert(String::from("api_base"), Json::Text(api_base.to_owned()));
    CreateParams::from_map(map).map_err(|error| SdkHttpError::Invalid(error.to_string()))
}

fn create_call<'call>(call: &'call Call, params: &'call CreateParams) -> CreateCall<'call> {
    CreateCall {
        model: &call.model,
        input: &call.input,
        optional_params: &call.optional_params,
        custom_llm_provider: PROVIDER,
        params,
        extra_headers: call.extra_headers.as_ref(),
        extra_body: call.extra_body.as_ref(),
        timeout: call.timeout_seconds,
        fake_stream: call.fake_stream.as_ref(),
        litellm_metadata: call.litellm_metadata.as_ref(),
        shared_session: None,
    }
}

// Only the callbacks the case asks for exist, so capability detection sees exactly what it registered
fn callbacks(call: &Call) -> Result<Vec<CallbackValue>, SdkHttpError> {
    let mut registered = Vec::new();
    if call.pre_hook.is_some() {
        registered.push(CallbackValue::Object(PolicyCallback {
            name: String::from("scripted-hook"),
            overrides_pre_hook: true,
            agentic: None,
        }));
    }
    if call.agentic {
        registered.push(CallbackValue::Object(PolicyCallback {
            name: String::from("scripted"),
            overrides_pre_hook: false,
            agentic: Some(Arc::new(Scripted {
                result: scripted_result(call.agentic_result.as_ref())?,
            })),
        }));
    }
    Ok(registered)
}

// A null scripted result is the hook answering None, which keeps the initial response
fn scripted_result(result: Option<&Json>) -> Result<Option<Box<RawValue>>, SdkHttpError> {
    let Some(value) = result.filter(|value| !matches!(value, Json::Null)) else {
        return Ok(None);
    };
    let text = serde_json::to_string(value).map_err(SdkHttpError::Serialize)?;
    RawValue::from_string(text)
        .map(Some)
        .map_err(SdkHttpError::ResultParse)
}

// A provider failure is what both sides can observe; a betterlite-only failure is not a parity answer
fn outcome(result: Result<CreateOutcome, CreateError>) -> Result<Outcome, SdkHttpError> {
    match result {
        Ok(CreateOutcome::Value(raw)) => Ok(Outcome {
            result: Some(serde_json::from_str(raw.get()).map_err(SdkHttpError::ResultParse)?),
            failure: None,
            stream: None,
        }),
        Ok(CreateOutcome::Live(live)) => Ok(streamed(StreamKind::Live, live.setup, None)),
        Ok(CreateOutcome::Synthetic(synthetic)) => {
            let response = to_raw_value(&synthetic.response).map_err(SdkHttpError::Serialize)?;
            Ok(streamed(
                StreamKind::Synthetic,
                synthetic.setup,
                Some(serde_json::from_str(response.get()).map_err(SdkHttpError::ResultParse)?),
            ))
        }
        Err(CreateError::Provider(error)) => Ok(Outcome {
            result: None,
            failure: Some(Failure {
                status: error.status_code,
                synthesized: error.status_code_is_synthesized,
                message: error.message,
                headers: error.headers,
            }),
            stream: None,
        }),
        Err(other) => Err(SdkHttpError::Invalid(other.to_string())),
    }
}

fn streamed(kind: StreamKind, setup: StreamSetup, response: Option<Json>) -> Outcome {
    Outcome {
        result: None,
        failure: None,
        stream: Some(Stream {
            kind,
            setup: setup_of(setup),
            response,
        }),
    }
}

fn setup_of(setup: StreamSetup) -> Setup {
    Setup {
        model: setup.model,
        custom_llm_provider: setup.custom_llm_provider,
        litellm_metadata: setup.litellm_metadata,
        request_data: setup.request_data,
        call_type: setup.call_type,
    }
}
