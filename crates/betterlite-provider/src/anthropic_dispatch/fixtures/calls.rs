use std::sync::PoisonError;
use std::time::Duration;

use betterlite_anthropic::batches::handler::RetrieveRequest;
use betterlite_anthropic::count_tokens::wire::{CountTokensBody, CountTokensRequest};
use betterlite_anthropic::files::request::{FileCall, ListCall, Upload};
use betterlite_context::chat_config_selection::table::PROVIDER_CONFIG_ROWS;
use betterlite_models::decisions::Json;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use http::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::super::base::{Globals, Surface};
use super::super::completion::{CompletionCall, CompletionSettings};
use super::super::config::AnthropicConfigKind;
use super::super::operations::{DispatchDeps, Operation, Outcome};
use super::super::{CallLogging, DispatchError, dispatch};
use super::recording::{Failure, Recorder, Transport, json};
use super::runtime::logging;
use super::{Action, FixtureError, Input, Observation};

pub(super) async fn run(input: &Input) -> Result<Observation, FixtureError> {
    let services = super::services::Services::from(input);
    let globals = Globals {
        api_base: input.global_base.clone(),
        anthropic_key: input.anthropic_key.clone(),
        api_key: input.global_key.clone(),
    };
    let completion = CompletionSettings {
        globals: &globals,
        environment: &input.environment,
        custom_prompt_dict: &input.global_prompt_dict,
        disable_suffix: input.settings.disable_suffix,
        selection: selection(input),
    };
    let mut handler = Recorder(input);
    let mut deps = DispatchDeps {
        completion: &completion,
        handler: &mut handler,
        count: &services.count,
        batches: &services.batches,
        files: &services.files,
        file_client: &services.client,
    };
    invoke(input, &mut deps).await
}

fn selection(
    input: &Input,
) -> Option<betterlite_context::chat_config_selection::tags::ChatConfigTag> {
    input
        .provider
        .parse::<betterlite_models::identity::provider_id::ProviderId>()
        .ok()
        .and_then(|provider| {
            PROVIDER_CONFIG_ROWS
                .direct
                .iter()
                .find(|(id, _tag)| *id == provider)
                .map(|(_id, tag)| *tag)
        })
        .filter(|_tag| input.settings.config_present)
}

async fn invoke(
    input: &Input,
    deps: &mut DispatchDeps<'_, Transport, Recorder<'_>>,
) -> Result<Observation, FixtureError> {
    let headers = headers(input)?;
    let file = FileData::Tuple3 {
        filename: Some("input.jsonl".to_owned()),
        content: FileBody::Bytes(input.file.as_bytes().to_vec()),
        content_type: Some("application/jsonl".to_owned()),
    };
    let messages = Json::List(input.messages.clone());
    let missing_key = SecretString::from("");
    let operation = operation(input, &headers, &file, &messages, &missing_key);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?;
    let (mut state, env) = logging(&redactor)?;
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let outcome = dispatch(operation, deps, &mut log).await;
    let result = outcome_value(outcome)?;
    let request = deps
        .files
        .transport
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(observation(result, &state, request))
}

fn observation(
    result: Json,
    state: &betterlite_logging::logging_state::LoggingState,
    request: Option<super::Recorded>,
) -> Observation {
    let post_call = state.model_call_details.get("log_event_type")
        == Some(&Json::Text("post_api_call".to_owned()));
    let post_matches = !post_call
        || state.model_call_details.get("original_response")
            == Some(&Json::Text(betterlite_logging::python_json::dumps(&result)));
    Observation {
        result,
        post_call,
        post_matches,
        request,
    }
}

fn headers(input: &Input) -> Result<HeaderMap, FixtureError> {
    let mut headers = HeaderMap::new();
    for (name, value) in &input.headers {
        headers.insert(HeaderName::try_from(name.as_str())?, {
            let mut header = HeaderValue::try_from(value.expose_secret())?;
            header.set_sensitive(true);
            header
        });
    }
    Ok(headers)
}

impl From<&Input> for CompletionCall<String, String> {
    fn from(input: &Input) -> Self {
        Self {
            model: input.model.clone(),
            provider: input.provider.clone(),
            messages: input.messages.clone(),
            model_response: betterlite_anthropic::response::wire::ResponseEnvelope::new(
                "chatcmpl-input".to_owned(),
            ),
            optional_params: input.optional_params.clone(),
            litellm_params: input.litellm_params.clone(),
            headers: input.headers.clone(),
            api_key: input.api_key.clone(),
            api_base: input.api_base.clone(),
            custom_prompt_dict: input.custom_prompt_dict.clone(),
            stream: input.stream,
            acompletion: input.acompletion,
            client: input.client.clone(),
            shared_session: input.shared_session.clone(),
            encoding: input.encoding.clone(),
            timeout: Duration::from_secs(5),
        }
    }
}

// The closed fixture input maps one-to-one to the production operation table.
#[rustfmt::skip]
fn operation<'call>(input: &'call Input, headers: &'call HeaderMap, file: &'call FileData, messages: &'call Json, missing_key: &'call SecretString) -> Operation<'call, String, String> {
    let file_call = FileCall { file_id: &input.id, headers };
    match input.action {
        Action::Messages => Operation::Completion(Surface::Messages, Box::new(CompletionCall::from(input))),
        Action::Legacy => Operation::Completion(Surface::Legacy, Box::new(CompletionCall::from(input))),
        Action::Count => Operation::Count(CountTokensRequest {
            body: CountTokensBody { model: &input.model, messages, tools: input.tools.as_ref(), system: input.system.as_ref(), optional_params: &input.optional_params },
            api_key: input.api_key.as_ref().unwrap_or(missing_key), api_base: input.api_base.as_deref(), timeout: None,
        }),
        Action::BatchCreate => Operation::BatchCreate,
        Action::BatchRetrieve => Operation::BatchRetrieve(RetrieveRequest { batch_id: &input.id, api_base: input.api_base.as_deref(), api_key: input.api_key.as_ref(), timeout: None, max_retries: None }),
        Action::BatchResults => Operation::BatchResults(file_call),
        Action::Upload => Operation::Upload(Upload { file: Some(file), purpose: input.purpose.as_deref(), boundary: "oracle-boundary".to_owned(), headers }),
        Action::Retrieve => Operation::Retrieve(file_call),
        Action::List => Operation::List(ListCall { purpose: input.purpose.as_deref(), headers }),
        Action::Delete => Operation::Delete(file_call),
        Action::Content => Operation::Content(file_call),
        Action::ConfigUrl => Operation::ConfigUrl(&input.id),
    }
}

fn outcome_value(
    outcome: Result<Outcome<Json>, DispatchError<Failure>>,
) -> Result<Json, FixtureError> {
    match outcome {
        Ok(Outcome::Completion(reply)) => Ok(reply),
        Ok(Outcome::Count(reply)) => json(&reply),
        Ok(Outcome::Batch(reply)) => json(&BatchView {
            id: &reply.id,
            status: &reply.status,
            output_file_id: &reply.output_file_id,
            request_counts: &reply.request_counts,
        }),
        Ok(Outcome::File(reply)) => json(&reply),
        Ok(Outcome::Files(reply)) => json(&reply),
        Ok(Outcome::Deleted(reply)) => json(&reply),
        Ok(Outcome::Content(reply)) => Ok(Json::Text(String::from_utf8(reply.body)?)),
        Ok(Outcome::Config(kind)) => Ok(Json::Text(
            match kind {
                AnthropicConfigKind::Anthropic => "AnthropicConfig",
                AnthropicConfigKind::AnthropicBatches => "AnthropicBatchesConfig",
            }
            .to_owned(),
        )),
        Err(error) => json(&FailureView::from(error)),
    }
}

#[derive(serde::Serialize)]
struct FailureView {
    error: bool,
    status: Option<u16>,
}

impl From<DispatchError<Failure>> for FailureView {
    fn from(error: DispatchError<Failure>) -> Self {
        Self {
            error: true,
            status: match error {
                DispatchError::Completion(Failure::Status(status)) => Some(status),
                DispatchError::Count(error) => Some(error.status_code()),
                DispatchError::Selection(_)
                | DispatchError::Batch(_)
                | DispatchError::File(_)
                | DispatchError::Completion(Failure::Capture(_)) => None,
            },
        }
    }
}

#[derive(serde::Serialize)]
struct BatchView<'reply> {
    id: &'reply str,
    status: &'reply betterlite_openai::batches::records::BatchStatus,
    output_file_id: &'reply Option<String>,
    request_counts: &'reply Option<betterlite_openai::batches::records::RequestCounts>,
}
