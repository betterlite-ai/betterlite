use crate::helpers::{HelperError, HelperResult};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody, merge_query};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::client::{ClientKind, ClientOptions, SdkClient};
use betterlite_openai::assistants::error::AssistantsError;
use betterlite_openai::assistants::operations::AssistantsApi;
use betterlite_openai::assistants::transport::AssistantsTransport;
use input::{AssistantsCall, ClientFields, KindName, OptionFields, Reply};
use reqwest::header::HeaderMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use serde_json::{Value, json};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

const PAGE: &str = r#"{"object":"list","data":[],"first_id":null,"last_id":null,"has_more":false}"#;
const ASSISTANT: &str = r#"{"id":"asst_1","object":"assistant","created_at":1,"model":"gpt-4o","tools":[],"tool_resources":null}"#;
const DELETED: &str = r#"{"id":"asst_1","deleted":true,"object":"assistant.deleted"}"#;

struct Wire {
    seen: Mutex<Vec<Value>>,
    status: u16,
    body: String,
}

impl Wire {
    fn replying(given: Option<&Reply>, canned: &str) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            status: given.map_or(200, |reply| reply.status),
            body: given.map_or_else(|| canned.to_owned(), |reply| reply.body.clone()),
        })
    }
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

impl AssistantsTransport for Wire {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let mut url = request.url.clone();
        merge_query(&mut url, &request.query);
        let body = match &request.body {
            RequestBody::Json(bytes) => serde_json::from_slice(bytes).unwrap_or(Value::Null),
            RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
                Value::Null
            }
        };
        let content_type = header(&request.headers, "content-type")
            .or_else(|| (!body.is_null()).then(|| String::from("application/json")));
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "method": request.method.as_str(),
                "url": url.as_str(),
                "authorization": header(&request.headers, "authorization"),
                "openai-organization": header(&request.headers, "openai-organization"),
                "openai-beta": header(&request.headers, "openai-beta"),
                "content-type": content_type,
                "body": body,
            }));
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers: HeaderMap::new(),
            body: self.body.clone().into_bytes(),
        }))
    }
}

fn invalid_input(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn client_options(fields: &OptionFields) -> Result<ClientOptions, Box<HelperError>> {
    let timeout = fields
        .timeout
        .map(Duration::try_from_secs_f64)
        .transpose()
        .map_err(|error| invalid_input(error.to_string()))?;
    Ok(ClientOptions {
        api_key: fields.api_key.clone().map(SecretString::from),
        api_base: fields.api_base.clone(),
        timeout,
        max_retries: fields.max_retries,
        organization: fields.organization.clone(),
    })
}

const fn kind_of(name: KindName) -> ClientKind {
    match name {
        KindName::Sync => ClientKind::Sync,
        KindName::Async => ClientKind::Async,
    }
}

// What the Python helper reports for the errors the OpenAI SDK raises.
fn failure(error: &AssistantsError) -> Result<Value, Box<HelperError>> {
    match error {
        AssistantsError::Status { status, .. } => Ok(json!({"kind": "status", "status": status})),
        AssistantsError::MissingApiKey => Ok(json!({"kind": "configuration"})),
        AssistantsError::EmptyAssistantId | AssistantsError::DotSegment { .. } => {
            Ok(json!({"kind": "invalid_assistant_id"}))
        }
        AssistantsError::BaseUrl { .. }
        | AssistantsError::Header { .. }
        | AssistantsError::Encode { .. }
        | AssistantsError::Transport { .. }
        | AssistantsError::Malformed { .. }
        | AssistantsError::Truncated { .. }
        | AssistantsError::Shape { .. } => Err(invalid_input(error.to_string())),
    }
}

fn outcome(wire: &Wire, result: Result<Value, AssistantsError>) -> HelperResult {
    let requests = wire
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    match result {
        Ok(value) => Ok(json!({"requests": requests, "result": value})),
        Err(error) => Ok(json!({"requests": requests, "error": failure(&error)?})),
    }
}

async fn exchange<T, F, C>(
    given: Option<&Reply>,
    canned: &str,
    options: &OptionFields,
    call: C,
) -> HelperResult
where
    T: Serialize,
    F: Future<Output = Result<T, AssistantsError>>,
    C: FnOnce(AssistantsApi<Wire>, ClientOptions) -> F,
{
    let wire = Wire::replying(given, canned);
    let api = AssistantsApi {
        transport: Arc::clone(&wire),
    };
    let result = call(api, client_options(options)?).await;
    outcome(
        &wire,
        result.map(|value| serde_json::to_value(value).unwrap_or(Value::Null)),
    )
}

fn shown(client: &SdkClient<Wire>) -> Value {
    let kind = match client.kind {
        ClientKind::Sync => "sync",
        ClientKind::Async => "async",
    };
    let mut fields = serde_json::Map::new();
    fields.insert(String::from("kind"), json!(kind));
    let given = [
        (
            "api_key",
            client
                .api_key
                .as_ref()
                .map(|key| json!(key.expose_secret())),
        ),
        ("base_url", client.base_url.as_ref().map(|base| json!(base))),
        (
            "timeout",
            client.timeout.map(|timeout| json!(timeout.as_secs_f64())),
        ),
        (
            "max_retries",
            client.max_retries.map(|retries| json!(retries)),
        ),
        (
            "organization",
            client.organization.as_ref().map(|name| json!(name)),
        ),
    ];
    for (name, setting) in given {
        if let Some(present) = setting {
            fields.insert(String::from(name), present);
        }
    }
    Value::Object(fields)
}

fn client_arguments(fields: &ClientFields) -> HelperResult {
    let wire = Wire::replying(None, PAGE);
    let api = AssistantsApi {
        transport: Arc::clone(&wire),
    };
    let supplied = fields.supplied.as_ref().map(|supplied| SdkClient {
        kind: kind_of(supplied.kind),
        api_key: None,
        base_url: None,
        timeout: None,
        max_retries: None,
        organization: supplied.organization.clone(),
        transport: Arc::clone(&wire),
    });
    let client = api.get_openai_client(
        kind_of(fields.kind),
        client_options(&fields.options)?,
        supplied,
    );
    Ok(json!({ "client": shown(&client) }))
}

fn invoke(input: &AssistantsCall) -> HelperResult {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| invalid_input(error.to_string()))?;
    match input {
        AssistantsCall::ListAssistants(fields) => runtime.block_on(exchange(
            fields.reply.as_ref(),
            PAGE,
            &fields.options,
            |api, options| async move {
                if fields.direct {
                    api.async_get_assistants(options, None, &fields.params)
                        .await
                } else {
                    api.get_assistants(options, None, fields.flag, &fields.params)
                        .await
                }
            },
        )),
        AssistantsCall::CreateAssistant(fields) => runtime.block_on(exchange(
            fields.reply.as_ref(),
            ASSISTANT,
            &fields.options,
            |api, options| async move {
                api.create_assistants(options, &fields.data, None, fields.flag)
                    .await
            },
        )),
        AssistantsCall::DeleteAssistant(fields) => runtime.block_on(exchange(
            fields.reply.as_ref(),
            DELETED,
            &fields.options,
            |api, options| async move {
                api.delete_assistant(options, &fields.assistant_id, None, fields.flag)
                    .await
            },
        )),
        AssistantsCall::ClientArguments(fields) => client_arguments(fields),
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = AssistantsCall;

    fn run(input: &Self::Input) -> HelperResult {
        invoke(input)
    }
}

mod input;
mod tests;
