use std::error::Error;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_core::api::genai_http::error::GenAiHttpError;
use betterlite_core::api::genai_http::service::{AsyncStream, SyncStream};
use betterlite_core::google_genai::streaming::{AsyncGoogleGenAiStream, GoogleGenAiStream};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::HiddenParams;
use betterlite_vertex::google_native_content::response::GenerateContentResponse;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::wire::Source;

// The answer the Python helper builds: the request, pre-call, response or iterator metadata, and base64 events
pub(super) struct Run<'run> {
    pub(super) source: &'run Source,
    pub(super) state: &'run LoggingState,
}

impl Run<'_> {
    pub(super) fn unary(&self, response: &GenerateContentResponse) -> HelperResult {
        let pre_call = pre_call_view(self.state)?;
        Ok(json!({
            "request": self.source.view()?,
            "pre_call": pre_call,
            "response": serde_json::to_value(response).or_value_error()?,
            "httpx_response": http_view(self.state),
            "hidden_params": Value::Null,
            "events": Vec::<String>::new(),
        }))
    }

    pub(super) fn sync_stream(&self, mut stream: SyncStream) -> HelperResult {
        let pre_call = pre_call_view(self.state)?;
        let http = http_view(self.state);
        let events = sync_events(&mut stream.iterator)?;
        self.stream(pre_call, http, &stream.metadata.hidden_params, events)
    }

    pub(super) async fn async_stream(&self, mut stream: AsyncStream) -> HelperResult {
        let pre_call = pre_call_view(self.state)?;
        let http = http_view(self.state);
        let events = async_events(&mut stream.iterator).await?;
        self.stream(pre_call, http, &stream.metadata.hidden_params, events)
    }

    fn stream(
        &self,
        pre_call: Value,
        http: Value,
        hidden: &HiddenParams,
        events: Vec<String>,
    ) -> HelperResult {
        Ok(json!({
            "request": self.source.view()?,
            "pre_call": pre_call,
            "response": Value::Null,
            "httpx_response": http,
            "hidden_params": serde_json::to_value(hidden).or_value_error()?,
            "events": events,
        }))
    }
}

fn sync_events(iterator: &mut GoogleGenAiStream) -> Result<Vec<String>, Box<HelperError>> {
    let mut events = Vec::new();
    for event in iterator.by_ref() {
        events.push(BASE64.encode(event.or_value_error()?));
    }
    Ok(events)
}

async fn async_events(
    iterator: &mut AsyncGoogleGenAiStream,
) -> Result<Vec<String>, Box<HelperError>> {
    let mut events = Vec::new();
    while let Some(event) = iterator.next_event().await.or_value_error()? {
        events.push(BASE64.encode(event));
    }
    Ok(events)
}

fn pre_call_view(state: &LoggingState) -> Result<Value, Box<HelperError>> {
    let details = serde_json::to_value(&state.model_call_details).or_value_error()?;
    let args = field(&details, "additional_args")?;
    Ok(json!({
        "input": field(&details, "input")?,
        "api_base": field(args, "api_base")?,
        "headers": field(args, "headers")?,
        "complete_input_dict": field(args, "complete_input_dict")?,
    }))
}

// transform_generate_content_response keeps the raw reply on the logging state; a stream never runs it
fn http_view(state: &LoggingState) -> Value {
    match &state.httpx_response {
        None => Value::Null,
        Some(answer) => json!({
            "status": answer.status,
            "text": String::from_utf8_lossy(&answer.body),
        }),
    }
}

fn field<'value>(value: &'value Value, name: &str) -> Result<&'value Value, Box<HelperError>> {
    value.get(name).ok_or_else(|| {
        Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: format!("the logging state has no {name}"),
        })
    })
}

// _handle_error raises BaseLLMException with the provider's status, which is what LiteLLM's caller sees
pub(super) fn helper_error<Source>(error: GenAiHttpError<Source>) -> Box<HelperError>
where
    Source: Error + 'static,
{
    let message = error.to_string();
    match error {
        GenAiHttpError::Provider(exception) => Box::new(HelperError {
            class: "BaseLLMException".to_owned(),
            status: Some(exception.status_code),
            error_type: None,
            param: None,
            code: None,
            message: exception.message,
        }),
        GenAiHttpError::Client(_)
        | GenAiHttpError::Logging(_)
        | GenAiHttpError::Auth(_)
        | GenAiHttpError::Transform(_)
        | GenAiHttpError::BodyNotObject
        | GenAiHttpError::UnexpectedStream => Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message,
        }),
    }
}
