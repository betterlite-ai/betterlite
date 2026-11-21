use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_config::types::ProxyTrust;
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object as PromptObject;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::runtime::Builder;

use crate::foundry_dispatch::chat::Answer;
use crate::foundry_dispatch::claude;
use crate::foundry_dispatch::credentials::{Globals, no_credentials};
use crate::foundry_dispatch::fixtures::claude::{NOW, TIMEOUT};
use crate::prepared::deployment::Deployment;

pub const KEY: &str = "sk-foundry-fixture";

pub fn field<'value>(value: &'value JsonValue, name: &str) -> &'value JsonValue {
    let JsonValue::Object(fields) = value else {
        panic!("the value is a JSON object");
    };
    fields.get(name).expect("the field is present")
}

pub fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the runtime builds")
        .block_on(future)
}

pub fn supplied(status: u16, body: &str) -> Answer<'static> {
    Answer::Supplied {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub fn upstream(status: u16, body: &str) -> UpstreamResponse {
    UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub fn request(body: &str) -> ChatRequest {
    serde_json::from_str(body).expect("a chat request")
}

pub fn simple_request(model: &str) -> ChatRequest {
    request(&format!(
        r#"{{"model":"{model}","messages":[{{"role":"user","content":"hi"}}]}}"#
    ))
}

pub fn deployment(api_base: &str, api_key: Option<&str>) -> Deployment {
    let mut params: IndexMap<String, JsonValue> = IndexMap::new();
    params.insert(
        String::from("model"),
        JsonValue::Text(String::from("deployment")),
    );
    params.insert(
        String::from("api_base"),
        JsonValue::Text(api_base.to_owned()),
    );
    if let Some(key) = api_key {
        params.insert(String::from("api_key"), JsonValue::Text(key.to_owned()));
    }
    Deployment::from_params(params).expect("the deployment")
}

pub fn claude_call<'data>(
    messages: &'data [PromptObject],
    params: &'data PromptObject,
    key: &'data SecretString,
    acompletion: bool,
    stream: bool,
) -> claude::Call<'data> {
    claude::Call {
        model: "claude-sonnet-4-5",
        messages,
        optional_params: IndexMap::from([(
            String::from("temperature"),
            Json::Text(String::from("warm")),
        )]),
        litellm_params: params,
        caller_headers: IndexMap::new(),
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: Some(key),
        timeout: TIMEOUT,
        acompletion,
        stream,
        now: NOW,
        disable_add_prefix_to_prompt: false,
    }
}

pub fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the policy"),
    );
    ProxiedClient::outbound(&policy).expect("the client")
}

pub async fn loopback() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener");
    let address = listener.local_addr().expect("the address");
    (listener, format!("http://{address}/models"))
}

// Reads one request body and answers it, returning the raw request text
pub async fn serve_once(listener: TcpListener, body: &[u8], reply: &str) -> String {
    let (mut stream, _peer) = listener.accept().await.expect("the connection");
    let mut received = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !received.ends_with(body) {
        let read = stream.read(&mut chunk).await.expect("the request bytes");
        assert_ne!(read, 0, "the client closed before sending the body");
        received.extend_from_slice(chunk.get(..read).expect("the bytes read"));
    }
    stream.write_all(reply.as_bytes()).await.expect("the reply");
    String::from_utf8(received).expect("the request text")
}

pub struct Env {
    pub global_api_base: Option<String>,
    pub global_api_key: Option<SecretString>,
    pub environment_api_base: Option<String>,
    pub environment_api_key: Option<SecretString>,
    pub headers: Option<Headers>,
    pub credentials: DeploymentSettings,
    pub environment: BTreeMap<String, String>,
}

impl Default for Env {
    fn default() -> Self {
        Self::new()
    }
}

impl Env {
    #[must_use]
    pub fn new() -> Self {
        Self {
            global_api_base: None,
            global_api_key: None,
            environment_api_base: None,
            environment_api_key: None,
            headers: None,
            credentials: no_credentials(),
            environment: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn global_base(self, api_base: &str) -> Self {
        Self {
            global_api_base: Some(api_base.to_owned()),
            ..self
        }
    }

    #[must_use]
    pub fn global_key(self, api_key: &str) -> Self {
        Self {
            global_api_key: Some(SecretString::from(api_key)),
            ..self
        }
    }

    #[must_use]
    pub fn environment_base(self, api_base: &str) -> Self {
        Self {
            environment_api_base: Some(api_base.to_owned()),
            ..self
        }
    }

    #[must_use]
    pub fn environment_key(self, api_key: &str) -> Self {
        Self {
            environment_api_key: Some(SecretString::from(api_key)),
            ..self
        }
    }

    #[must_use]
    pub fn headers(self, headers: Headers) -> Self {
        Self {
            headers: Some(headers),
            ..self
        }
    }

    pub fn with_globals(api_base: &str, api_key: &str) -> Self {
        Self::new().global_base(api_base).global_key(api_key)
    }

    pub fn globals(&self) -> Globals<'_> {
        Globals {
            api_base: self.global_api_base.as_deref(),
            api_key: self.global_api_key.as_ref(),
            environment_api_base: self.environment_api_base.as_deref(),
            environment_api_key: self.environment_api_key.as_ref(),
            headers: self.headers.as_ref(),
            environment: &self.environment,
            credentials: &self.credentials,
            refresh_enabled: false,
            sources: &NoSources,
            token_provider: None,
        }
    }
}
