use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::speech_call::error::{SpeechCallError, SpeechException};
use betterlite_core::api::speech_call::service::SpeechDispatch;
use betterlite_core::api::speech_chat_bridge::{
    SpeechCompletionAnswer, SpeechCompletionCall, SpeechCompletionOutput, SpeechCompletions,
};
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::execution::ChatError;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::files::error::StatusError;
use betterlite_openai::files::records::BinaryContent;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;

use super::super::OpenaiChat;
use super::super::error::SdkHttpError;
use super::super::native;
use super::super::result::kinds::SdkResult;
use super::support::{self, CannedOperations, Fixture, StepConfigs};
use super::{BinaryValue, Call, Outcome, StepOutcome};

const DEFAULT_ORACLE_KEY: &str = "sk-oracle";
const DEFAULT_TIMEOUT_SECONDS: u64 = 6000;

// the completion seam the bridge calls: an OpenAI-compatible chat request against the mock
struct BridgeCompletions {
    api_base: String,
    api_key: String,
    timeout_seconds: u64,
    env: BTreeMap<String, String>,
}

impl SpeechCompletions for BridgeCompletions {
    type Error = SpeechCallError;

    fn speech_completion(
        &self,
        call: SpeechCompletionCall<'_>,
    ) -> impl Future<Output = Result<SpeechCompletionOutput, SpeechCallError>> + Send {
        let SpeechCompletionCall { kwargs, .. } = call;
        let api_base = self.api_base.clone();
        let env = self.env.clone();
        let api_key = self.api_key.clone();
        let timeout_seconds = self.timeout_seconds;
        async move {
            let model = projected_model(&kwargs)?;
            let request = bridge_request(&kwargs)?;
            let chat = OpenaiChat {
                model: model.clone(),
                messages: Vec::new(),
                api_key,
                timeout_seconds,
            };
            let executor = native::executor(&chat, &api_base, &env).map_err(|error| {
                SpeechCallError::Internal {
                    message: error.to_string(),
                }
            })?;
            let deployment =
                executor
                    .deployments
                    .first()
                    .ok_or_else(|| SpeechCallError::Internal {
                        message: format!("no deployment serves {model}"),
                    })?;
            match executor.send(deployment, &request).await {
                Ok(answer) => Ok(completion_output(answer.response)),
                Err(ChatError::Provider { response }) => Err(provider_error(&response)),
                Err(other) => Err(SpeechCallError::Internal {
                    message: SdkHttpError::Chat(Box::new(other)).to_string(),
                }),
            }
        }
    }
}

fn completion_output(response: ChatResponse) -> SpeechCompletionOutput {
    SpeechCompletionOutput::Response(Box::new(SpeechCompletionAnswer {
        response,
        hidden_params: HiddenParams::new(),
    }))
}

fn provider_error(response: &UpstreamResponse) -> SpeechCallError {
    let status = StatusError::from_response(response.status, &response.body);
    SpeechCallError::Provider {
        status: status.status,
        message: status.message,
        error_type: status.error_type,
        param: status.param,
        code: status.code,
    }
}

fn projected_model(kwargs: &IndexMap<String, Json>) -> Result<String, SpeechCallError> {
    match kwargs.get("model") {
        Some(Json::Text(model)) => Ok(model.clone()),
        Some(_) | None => Err(SpeechCallError::Internal {
            message: String::from("the projected model must be text"),
        }),
    }
}

fn bridge_request(kwargs: &IndexMap<String, Json>) -> Result<ChatRequest, SpeechCallError> {
    let mut wire = kwargs.clone();
    wire.shift_remove("headers");
    wire.shift_remove("custom_llm_provider");
    let body = serde_json::to_vec(&wire).map_err(|error| SpeechCallError::Internal {
        message: error.to_string(),
    })?;
    ChatRequest::parse(&body).map_err(|error| SpeechCallError::Internal {
        message: error.to_string(),
    })
}

pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let environment = case_environment(env, api_base);
    let settings = support::settings(&call.globals, &environment, api_base)?;
    let fixture = Fixture::new(&environment, support::credentials(&call.globals, api_base))?;
    let context = fixture.context();
    let transport = outbound_client(&environment)?;
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?;
    let mut outcomes = Vec::with_capacity(call.steps.len());
    for step in &call.steps {
        let native_call = support::speak(step, api_base)?;
        let completions = BridgeCompletions {
            api_base: api_base.to_owned(),
            api_key: step
                .api_key
                .clone()
                .unwrap_or_else(|| String::from(DEFAULT_ORACLE_KEY)),
            timeout_seconds: step.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
            env: environment.clone(),
        };
        let configs = StepConfigs { lyria: step.lyria };
        let operations = CannedOperations {
            body: step.operation_body.clone(),
        };
        let dispatch = Dispatch {
            settings: &settings,
            resolution: &context,
            transport: &transport,
            completions: &completions,
            configs: &configs,
            operations: &operations,
        };
        let mut logging = support::logging()?;
        let result = dispatch.speech(&native_call, &mut logging, &redactor).await;
        outcomes.push(step_outcome(result)?);
    }
    Ok(SdkResult::SpeechCall {
        value: Box::new(Outcome { steps: outcomes }),
    })
}

type Dispatch<'service> =
    SpeechDispatch<'service, ProxiedClient, BridgeCompletions, StepConfigs, CannedOperations>;

fn case_environment(env: &BTreeMap<String, String>, api_base: &str) -> BTreeMap<String, String> {
    env.iter()
        .map(|(name, value)| (name.clone(), support::harness(value, api_base)))
        .collect()
}

fn step_outcome(
    result: Result<BinaryContent, SpeechException>,
) -> Result<StepOutcome, SdkHttpError> {
    match result {
        Ok(content) => Ok(StepOutcome {
            value: Some(BinaryValue {
                status: content.status,
                body: content.body,
            }),
            failure: None,
        }),
        Err(error) => Ok(StepOutcome {
            value: None,
            failure: Some(support::failure(&error)?),
        }),
    }
}

fn outbound_client(env: &BTreeMap<String, String>) -> Result<ProxiedClient, SdkHttpError> {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
    );
    ProxiedClient::outbound(&policy).map_err(|error| SdkHttpError::Client(Box::new(error)))
}
