use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::gcs_mock::auth::{
    AccessToken, EnsureAccessToken, GcsMockAuth, GetTokenAndUrl, TokenAndUrl, VertexAuth as _,
    VertexAuthProvider,
};
use betterlite_logging::gcs_mock::client::{
    GcsHttpClient as _, GcsHttpResponse, create_mock_gcs_client,
};
use betterlite_logging::gcs_mock::request::GcsRequest;
use betterlite_logging::gcs_mock::settings::should_use_gcs_mock;
use secrecy::ExposeSecret as _;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod input;
mod recording;
mod tests;

use input::{AuthInput, Input, RequestInput};
use recording::{FakeAuth, RecordingClient, last_recorded, observed_body, observed_request};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> HelperResult {
    let enabled =
        should_use_gcs_mock(&input.environment).map_err(|error| failure("ValueError", error))?;
    let mut output = Map::new();
    output.insert("enabled".to_owned(), json!(enabled));
    if let Some(requests) = &input.requests {
        output.insert("requests".to_owned(), run_requests(input, requests).await?);
    }
    if let Some(auth) = &input.auth {
        output.insert("auth".to_owned(), run_auth(auth).await?);
    }
    Ok(Value::Object(output))
}

async fn run_requests(input: &Input, requests: &[RequestInput]) -> Result<Value, Box<HelperError>> {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let real = input
        .original
        .as_ref()
        .map(|original| RecordingClient::new(Arc::clone(&sent), original.status, &original.body));
    let client = create_mock_gcs_client(&input.environment, real)
        .map_err(|error| failure("ValueError", error))?;
    let mut observed = Vec::with_capacity(requests.len());
    for request in requests {
        let gcs_request = request.clone().into_request()?;
        let before = sent.lock().unwrap_or_else(PoisonError::into_inner).len();
        let response = client
            .send(gcs_request)
            .await
            .map_err(|error| failure("RuntimeError", error))?;
        let delegated = sent.lock().unwrap_or_else(PoisonError::into_inner).len() != before;
        let received = if delegated {
            Some(last_recorded(&sent).ok_or_else(|| {
                failure(
                    "RuntimeError",
                    "the original client did not record the request",
                )
            })?)
        } else {
            None
        };
        observed.push(observed_entry(&response, received.as_ref()));
    }
    Ok(Value::Array(observed))
}

fn observed_entry(response: &GcsHttpResponse, received: Option<&GcsRequest>) -> Value {
    let mut entry = Map::new();
    entry.insert("mocked".to_owned(), json!(received.is_none()));
    entry.insert("status".to_owned(), json!(response.status));
    entry.insert("body".to_owned(), observed_body(&response.body));
    if let Some(request) = received {
        entry.insert("received".to_owned(), observed_request(request));
    } else {
        entry.insert("url".to_owned(), Value::String(response.url.clone()));
        entry.insert(
            "elapsed_ms".to_owned(),
            json!(u64::try_from(response.elapsed.as_millis()).unwrap_or(u64::MAX)),
        );
    }
    Value::Object(entry)
}

struct Tokens {
    asynchronous: AccessToken,
    synchronous: AccessToken,
    token_and_url: TokenAndUrl,
}

async fn run_auth(input: &AuthInput) -> Result<Value, Box<HelperError>> {
    let mut auth = GcsMockAuth {
        real: FakeAuth,
        installed: false,
    };
    let mut installs = Vec::new();
    for _ in 0..input.installs {
        installs.push(json!(auth.install()));
    }
    let tokens = authenticate(&auth).await?;
    Ok(json!({
        "installs": installs,
        "async_token": {"token": tokens.asynchronous.token.expose_secret(), "project_id": tokens.asynchronous.project_id},
        "sync_token": {"token": tokens.synchronous.token.expose_secret(), "project_id": tokens.synchronous.project_id},
        "token_and_url": {"token": tokens.token_and_url.token.expose_secret(), "url": tokens.token_and_url.url},
    }))
}

async fn authenticate(auth: &GcsMockAuth<FakeAuth>) -> Result<Tokens, Box<HelperError>> {
    let ensure = EnsureAccessToken {
        credentials: None,
        project_id: None,
        custom_llm_provider: VertexAuthProvider::VertexAi,
    };
    let asynchronous = auth
        .ensure_access_token_async(ensure.clone())
        .await
        .map_err(|error| failure("RuntimeError", error))?;
    let synchronous = auth
        .ensure_access_token(ensure)
        .map_err(|error| failure("RuntimeError", error))?;
    let token = GetTokenAndUrl {
        model: "gcs-bucket".to_owned(),
        auth_header: None,
        vertex_credentials: None,
        vertex_project: None,
        vertex_location: None,
        gemini_api_key: None,
        stream: None,
        custom_llm_provider: VertexAuthProvider::VertexAi,
        api_base: None,
    };
    let token_and_url = auth
        .get_token_and_url(token)
        .map_err(|error| failure("RuntimeError", error))?;
    Ok(Tokens {
        asynchronous,
        synchronous,
        token_and_url,
    })
}

pub(crate) fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
