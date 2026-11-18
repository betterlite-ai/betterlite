mod execution;
mod fixture;
mod media;
mod response;
mod services;
mod wire;

use crate::anthropic_dispatch::base::Surface;
use crate::anthropic_dispatch::completion::CompletionCall;
use crate::anthropic_dispatch::native::ClientHandle;
use betterlite_anthropic::chat_call::CallStyle;
use betterlite_anthropic::request::error::RequestError;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_http::proxy::ProxiedClient;
use betterlite_tokenizer::engine::Engine;
use fixture::Fixture;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::task::JoinSet;
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(5);
const CHAT_REPLY: &str = r#"{"id":"msg_native","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"text","text":"native answer"},{"type":"tool_use","id":"tool_native","name":"bad_name","input":{}}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#;
const LEGACY_REPLY: &str = r#"{"completion":"legacy answer","stop_reason":"stop_sequence"}"#;
type Call = CompletionCall<ClientHandle<ProxiedClient>, Engine>;

#[derive(Default)]
pub(super) struct Observed {
    pub posts: usize,
    pub content: Option<String>,
    pub body: wire::Body,
    pub key: String,
    pub status: Option<u16>,
    pub logged: String,
    pub tool_name: Option<String>,
    pub preparation_error: Option<Box<RequestError<Infallible>>>,
}
struct Scenario<'input> {
    surface: Surface,
    style: CallStyle,
    key: bool,
    options: &'input str,
}

pub(super) fn run(
    surface: Surface,
    style: CallStyle,
    key: bool,
    tag: ChatConfigTag,
    options: &str,
) -> Observed {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut tasks = JoinSet::new();
        let answer = match surface {
            Surface::Messages => CHAT_REPLY,
            Surface::Legacy => LEGACY_REPLY,
        };
        tasks.spawn(wire::server(listener, answer));
        let client = ProxiedClient::outbound(&Arc::new(
            ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap(),
        ))
        .unwrap();
        let fixture = Fixture::from(client);
        let scenario = Scenario {
            surface,
            style,
            key,
            options,
        };
        let call = fixture.call(&scenario, base);
        let mut observed = fixture.invoke(surface, call, tag).await;
        if observed.posts == 1 {
            let (body, credential) = timeout(TIMEOUT, tasks.join_next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            observed.body = body;
            observed.key = credential;
        }
        fixture.client.drain(TIMEOUT).await.unwrap();
        observed
    })
}
