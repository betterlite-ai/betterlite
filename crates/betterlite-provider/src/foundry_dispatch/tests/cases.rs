use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_anthropic::chat_call::transport::ChatClients;
use betterlite_anthropic::chat_call::{ChatDeps, ChatOutcome};
use betterlite_foundry::agent_results::reply::AgentReply;
use betterlite_foundry::agent_runs::transport::AgentRunMethod;
use betterlite_foundry::embeddings::call::{EmbeddingMode, FoundryEmbeddingOptions};
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};
use betterlite_prompts::message_content::Content;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::support::{self, Env, KEY, claude_call, field, loopback, proxied_client, serve_once};
use crate::foundry_dispatch::chat::Answer;
use crate::foundry_dispatch::fixtures::agents::{
    Clock, Factory as AgentFactory, Resolver, Scripted, Step, Tally as AgentTally,
};
use crate::foundry_dispatch::fixtures::claude::{Harness, Mapper, Tally, Wire};
use crate::foundry_dispatch::fixtures::embedding::{Factory as EmbeddingFactory, Text};
use crate::foundry_dispatch::fixtures::logger::RecordingLogger;
use crate::foundry_dispatch::route::{self, Route};
use crate::foundry_dispatch::{
    AzureAi, AzureAiRequest, agents, chat, claude, credentials, embedding,
};
use crate::operation::{
    ChatCompletion, ChatOperation, OperationError, OperationRequest, OperationResult,
};
use crate::prepared::PreparedRequest;
use crate::prepared::deployment::Deployment;
use crate::provider::Adapter;

const REPLY: &str = r#"{"id":"chatcmpl-foundry","object":"chat.completion","created":1700000000,"model":"provider-model","choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const ANTHROPIC_REPLY: &str = r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#;
const BASE: &str = "https://res.services.ai.azure.com/models";

#[derive(Clone, Copy)]
enum Case {
    Chat,
    Embedding,
    Anthropic,
    Agent,
    AgentsMissingBase,
    ClaudeMissingBase,
    ClaudeUrlRewrite,
    CommandR,
    Http,
    PostCall,
}

#[rstest]
#[case::foundry_chat_dispatch(Case::Chat)]
#[case::foundry_embedding_dispatch(Case::Embedding)]
#[case::foundry_anthropic_dispatch(Case::Anthropic)]
#[case::foundry_agent_dispatch(Case::Agent)]
#[case::agents_missing_base(Case::AgentsMissingBase)]
#[case::claude_missing_base(Case::ClaudeMissingBase)]
#[case::claude_url_rewrite(Case::ClaudeUrlRewrite)]
#[case::command_r_tool_content(Case::CommandR)]
#[case::foundry_provider_dispatch_http(Case::Http)]
#[case::foundry_post_call(Case::PostCall)]
fn foundry_dispatch_cases(#[case] case: Case) {
    match case {
        Case::Chat => support::block_on(chat_case()),
        Case::Embedding => support::block_on(embedding_case()),
        Case::Anthropic => support::block_on(anthropic_case()),
        Case::Agent => support::block_on(agent_case()),
        Case::AgentsMissingBase => support::block_on(agents_missing_base()),
        Case::ClaudeMissingBase => support::block_on(claude_missing_base()),
        Case::ClaudeUrlRewrite => claude_url_rewrite(),
        Case::CommandR => command_r_case(),
        Case::Http => support::block_on(http_case()),
        Case::PostCall => support::block_on(post_call_case()),
    }
}

fn chat_call<'data>(
    request: &'data ChatRequest,
    deployment: &'data Deployment,
    model: &'data str,
    stream: bool,
) -> chat::Call<'data> {
    chat::Call {
        model,
        request,
        extra_headers: None,
        headers: None,
        api_base: Some(deployment.api_base.as_str()),
        api_key: deployment.api_key.as_ref(),
        deployment,
        stream,
    }
}

async fn chat_case() {
    // main.py resolves the agents route first, then a case-insensitive claude substring, then the chat route
    assert_eq!(route::select("agents/agent-claude"), Route::Agents);
    assert_eq!(route::select("CLAUDE-sonnet"), Route::Claude);
    assert_eq!(route::select("mistral-large"), Route::Chat);

    let env = Env::with_globals("https://global.example.test", "global-key");
    let deployment = support::deployment(BASE, Some(KEY));
    let request = support::simple_request("mistral-large");
    let call = chat_call(&request, &deployment, "mistral-large", false);
    let dispatch = chat::Dispatch {
        call,
        globals: env.globals(),
        answer: support::supplied(200, REPLY),
    };
    let mut logger = RecordingLogger::default();
    let outcome = dispatch
        .run(&mut logger)
        .await
        .expect("the chat dispatch runs");
    assert!(logger.calls.is_empty());
    let OperationResult::Chat(ChatCompletion::OpenAi(completion)) = outcome.result else {
        panic!("the chat dispatch returns a chat reply");
    };
    assert_eq!(completion.model, "azure_ai/provider-model");
    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("hello")
    );
    let PreparedRequest::Chat(prepared) = &outcome.prepared else {
        panic!("the chat call prepares a chat request");
    };
    assert_eq!(
        prepared.http.url.as_str(),
        "https://res.services.ai.azure.com/models/chat/completions"
    );
    // a Foundry host takes the key in api-key, as LiteLLM's azure v1 host rule says
    assert_eq!(
        prepared
            .http
            .headers
            .get("api-key")
            .and_then(|value| value.to_str().ok()),
        Some("sk-foundry-fixture")
    );
    let sent: JsonValue = serde_json::from_slice(&prepared.http.body).expect("the request body");
    assert_eq!(
        field(&sent, "model"),
        &JsonValue::Text(String::from("mistral-large"))
    );
    let JsonValue::Array(messages) = field(&sent, "messages") else {
        panic!("the sent messages are an array");
    };
    assert_eq!(
        field(messages.first().expect("a message"), "content"),
        &JsonValue::Text(String::from("hi"))
    );

    // the same context with no deployment key uses the global one through B549
    let deployment = support::deployment(BASE, None);
    let call = chat_call(&request, &deployment, "mistral-large", false);
    let dispatch = chat::Dispatch {
        call,
        globals: env.globals(),
        answer: support::supplied(200, REPLY),
    };
    let outcome = dispatch
        .run(&mut RecordingLogger::default())
        .await
        .expect("the chat dispatch runs");
    let PreparedRequest::Chat(prepared) = &outcome.prepared else {
        panic!("the chat call prepares a chat request");
    };
    assert_eq!(
        prepared
            .http
            .headers
            .get("api-key")
            .and_then(|value| value.to_str().ok()),
        Some("global-key")
    );

    // neither the agents nor the claude model reaches the chat leaf
    for (model, branch) in [
        ("agents/agent-claude", "agents"),
        ("claude-sonnet", "claude"),
    ] {
        let request = support::simple_request(model);
        let call = chat_call(&request, &deployment, model, false);
        let error = chat::prepare::prepare(&call, &env.globals())
            .err()
            .expect("the route is not chat");
        assert_eq!(
            error.to_string(),
            format!("the {branch} branch of the Foundry provider is not this chat call")
        );
    }
}

async fn embedding_case() {
    let text = Text {
        data: vec![EmbeddingRecord {
            object: String::from("embedding"),
            embedding: EmbeddingVector::Floats(vec![serde_json::Number::from(1_u32)]),
            index: 0,
        }],
        model: Some(String::from("text-embedding")),
        sent: Mutex::new(Vec::new()),
    };
    let sent = Arc::new(Mutex::new(Vec::new()));
    let factory = EmbeddingFactory {
        status: 200,
        image_body: String::from(
            r#"{"object":"list","data":[{"object":"embedding","embedding":[0.25],"index":0}],"model":"cohere-embed"}"#,
        ),
        sent: Arc::clone(&sent),
    };
    let input = [String::from("aGVsbG8="), String::from("plain text")];
    let params = PromptObject::new();
    let call = embedding::Call {
        model: "text-embedding",
        input: &input,
        options: FoundryEmbeddingOptions::default(),
        api_base: None,
        api_key: None,
        params: &params,
        timeout: Duration::from_secs(5),
        mode: EmbeddingMode::Async,
    };
    let env = Env::new()
        .environment_base("https://res.services.ai.azure.com")
        .environment_key(KEY);
    let response = embedding::dispatch(&call, &env.globals(), None, &factory, &text)
        .await
        .expect("the embedding dispatch runs");
    assert_eq!(response.data.len(), 2);
    assert_eq!(response.model.as_deref(), Some("text-embedding"));

    // the image half of the input runs through the image transport with the same resolved base and key
    {
        let images = sent.lock().expect("the image requests");
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].url,
            "https://res.services.ai.azure.com/models/images/embeddings"
        );
        let bearer = format!("Bearer {KEY}");
        assert!(
            images[0]
                .headers
                .iter()
                .any(|(name, value)| name == "authorization" && value == &bearer)
        );
        let image_body: JsonValue = serde_json::from_str(&images[0].body).expect("the image body");
        let JsonValue::Array(image_input) = field(&image_body, "input") else {
            panic!("the image input is an array");
        };
        assert_eq!(
            field(image_input.first().expect("one image"), "image"),
            &JsonValue::Text(String::from("aGVsbG8="))
        );
    }

    // the text half received the Foundry route base and the same key
    {
        let texts = text.snapshot();
        assert_eq!(texts.len(), 1);
        assert_eq!(
            texts[0].api_base.as_deref(),
            Some("https://res.services.ai.azure.com/models")
        );
        assert_eq!(texts[0].api_key.as_deref(), Some(KEY));
    }

    // an explicit key beats the global and environment values
    let explicit = SecretString::from("explicit-key");
    let global_env = Env::new().global_key("global-key").environment_key(KEY);
    let call = embedding::Call {
        api_key: Some(&explicit),
        ..call
    };
    embedding::dispatch(&call, &global_env.globals(), None, &factory, &text)
        .await
        .expect("the embedding dispatch runs");
    assert_eq!(text.snapshot()[1].api_key.as_deref(), Some("explicit-key"));

    // an explicit base beats the environment value for both halves
    let explicit_base = embedding::Call {
        api_base: Some("https://explicit.example.test"),
        ..call
    };
    embedding::dispatch(&explicit_base, &env.globals(), None, &factory, &text)
        .await
        .expect("the explicit base dispatch runs");
    assert_eq!(
        text.snapshot()[2].api_base.as_deref(),
        Some("https://explicit.example.test/models")
    );

    // an azure_ad_token parameter fills the key slot once every key string is unset
    let empty = Env::new();
    let entra_params: PromptObject = serde_json::from_str(r#"{"azure_ad_token":"entra-token"}"#)
        .expect("the entra token parameter");
    let token = credentials::resolve_embedding_key(None, &entra_params, &empty.globals())
        .expect("the entra token resolves")
        .expect("the entra token fills the key slot");
    assert_eq!(token.expose_secret().as_str(), "entra-token");
}

async fn anthropic_case() {
    let message: PromptObject =
        serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("a message");
    let messages = [message];
    let params = PromptObject::new();
    let key = SecretString::from(KEY);
    let env = Env::default();
    let harness = Harness::new().expect("the harness");

    let wire = Wire::new(200, ANTHROPIC_REPLY);
    let mut state = harness.state().expect("the logging state");
    let env_values = harness.env();
    let mut logger = RecordingLogger::default();
    let outcome = claude::Dispatch {
        call: claude_call(&messages, &params, &key, false, false),
        globals: env.globals(),
        deps: ChatDeps {
            config: &Mapper,
            clients: ChatClients {
                supplied: None,
                pooled: &wire,
            },
            state: &mut state,
            env: &env_values,
            stream_logger: harness.quiet(),
            counter: Tally,
            ids: harness.quiet(),
            max_event_bytes: 1_048_576,
            max_error_body_bytes: 1_048_576,
        },
    }
    .run(&mut logger)
    .await
    .expect("the claude dispatch runs");
    let ChatOutcome::Reply(envelope) = outcome else {
        panic!("the buffered call replies");
    };
    assert_eq!(envelope.model.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(
        envelope.choices[0].message.content.as_deref(),
        Some("hello")
    );
    assert!(logger.calls.is_empty());
    {
        let posts = wire.recorded();
        assert_eq!(
            posts[0].url.as_str(),
            "https://res.services.ai.azure.com/anthropic/v1/messages"
        );
        assert_eq!(
            posts[0]
                .headers
                .get("api-key")
                .and_then(|value| value.to_str().ok()),
            Some(KEY)
        );
    }

    // acompletion alone logs the buffered reply, and the async mapper carries the optional params
    let async_wire = Wire::new(200, ANTHROPIC_REPLY);
    let mut async_state = harness.state().expect("the logging state");
    let async_env = harness.env();
    let mut async_logger = RecordingLogger::default();
    let async_outcome = claude::Dispatch {
        call: claude_call(&messages, &params, &key, true, false),
        globals: env.globals(),
        deps: ChatDeps {
            config: &Mapper,
            clients: ChatClients {
                supplied: None,
                pooled: &async_wire,
            },
            state: &mut async_state,
            env: &async_env,
            stream_logger: harness.quiet(),
            counter: Tally,
            ids: harness.quiet(),
            max_event_bytes: 1_048_576,
            max_error_body_bytes: 1_048_576,
        },
    }
    .run(&mut async_logger)
    .await
    .expect("the async buffered dispatch runs");
    assert!(matches!(async_outcome, ChatOutcome::Reply(_)));
    assert_eq!(async_logger.calls.len(), 1);
    let expected_report: Json =
        serde_json::from_str(r#"{"model":"claude-sonnet-4-5","content":"hello"}"#)
            .expect("the reply report");
    assert_eq!(async_logger.calls[0].original_response, expected_report);
    {
        let async_posts = async_wire.recorded();
        let async_post: JsonValue =
            serde_json::from_slice(&async_posts[0].body).expect("the async body");
        assert_eq!(
            field(&async_post, "temperature"),
            &JsonValue::Text(String::from("warm"))
        );
    }

    // an async call with stream opens the stream and logs it, without additional_args headers
    let stream_wire = Wire::new(200, "");
    stream_wire.lines = vec![String::from(
        "event: message_stop\ndata: {\"type\":\"message_stop\"}",
    )];
    let mut state = harness.state().expect("the logging state");
    let env_values = harness.env();
    let mut logger = RecordingLogger::default();
    let outcome = claude::Dispatch {
        call: claude_call(&messages, &params, &key, true, true),
        globals: env.globals(),
        deps: ChatDeps {
            config: &Mapper,
            clients: ChatClients {
                supplied: None,
                pooled: &stream_wire,
            },
            state: &mut state,
            env: &env_values,
            stream_logger: harness.quiet(),
            counter: Tally,
            ids: harness.quiet(),
            max_event_bytes: 1_048_576,
            max_error_body_bytes: 1_048_576,
        },
    }
    .run(&mut logger)
    .await
    .expect("the claude stream dispatch runs");
    assert!(matches!(outcome, ChatOutcome::Stream(_)));
    assert_eq!(logger.calls.len(), 1);
    assert!(logger.calls[0].headers.is_none());

    // a provider failure reaches the caller without a post_call on this branch
    // a valid reply body behind the failing status proves the transport reports the status, not the body
    let failing = Wire::new(500, ANTHROPIC_REPLY);
    let mut state = harness.state().expect("the logging state");
    let env_values = harness.env();
    let mut logger = RecordingLogger::default();
    let result = claude::Dispatch {
        call: claude_call(&messages, &params, &key, false, false),
        globals: env.globals(),
        deps: ChatDeps {
            config: &Mapper,
            clients: ChatClients {
                supplied: None,
                pooled: &failing,
            },
            state: &mut state,
            env: &env_values,
            stream_logger: harness.quiet(),
            counter: Tally,
            ids: harness.quiet(),
            max_event_bytes: 1_048_576,
            max_error_body_bytes: 1_048_576,
        },
    }
    .run(&mut logger)
    .await;
    assert!(result.is_err());
    assert!(logger.calls.is_empty());
}

fn agent_messages() -> Vec<PromptObject> {
    vec![serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("a message")]
}

fn agent_step(status: u16, body: &str) -> Step {
    Step::new(status, body)
}

fn agent_reply() -> AgentReply {
    AgentReply::default()
}

async fn agent_case() {
    let script = Arc::new(Scripted::new(vec![
        agent_step(201, r#"{"id":"thread_1"}"#),
        agent_step(201, r#"{"id":"message_1"}"#),
        agent_step(201, r#"{"id":"run_1"}"#),
        agent_step(200, r#"{"status":"in_progress"}"#),
        agent_step(200, r#"{"status":"completed"}"#),
        agent_step(
            200,
            r#"{"data":[{"role":"assistant","content":[{"type":"text","text":{"value":"hello"}}]}]}"#,
        ),
    ]));
    let factory = AgentFactory {
        script: Arc::clone(&script),
    };
    let clock = Clock::default();
    let tally = AgentTally;
    let resolver = Resolver {
        token: Some(String::from("entra-token")),
        seen: Mutex::new(Vec::new()),
    };
    let messages = agent_messages();
    let optional = PromptObject::new();
    let params = PromptObject::new();
    let env = Env::new().headers(BTreeMap::from([(
        String::from("x-team"),
        String::from("alpha"),
    )]));
    let call = agents::Call {
        model: "agents/my-agent",
        messages: &messages,
        optional_params: &optional,
        litellm_params: &params,
        headers: None,
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        acompletion: false,
        stream: false,
        reply: agent_reply(),
    };
    let dispatch = agents::Dispatch {
        call,
        globals: env.globals(),
        deps: agents::Deps {
            factory: &factory,
            sleeper: &clock,
            counter: &tally,
            resolver: &resolver,
        },
    };
    let outcome = dispatch.run().await.expect("the agent dispatch runs");
    let agents::Outcome::Reply(reply) = outcome else {
        panic!("the non-stream agent call replies");
    };
    assert_eq!(reply.model.as_deref(), Some("agents/my-agent"));
    assert_eq!(reply.choices[0].message.content, "hello");
    assert_eq!(
        reply
            .hidden_params
            .as_ref()
            .and_then(|params| params.get("thread_id")),
        Some(&PromptJson::Text(String::from("thread_1")))
    );
    assert_eq!(
        reply.usage.as_ref().map(|usage| (
            usage.prompt_tokens,
            usage.completion_tokens,
            usage.total_tokens
        )),
        Some((1, 5, 6))
    );

    let requests = script.requests();
    assert_eq!(requests.len(), 6);
    // the poll that finds the run unfinished waits once in the supplied clock
    assert_eq!(clock.waits.lock().expect("the poll waits").len(), 1);
    assert_eq!(requests[0].method, AgentRunMethod::Post);
    assert_eq!(
        requests[0].url,
        "https://res.services.ai.azure.com/threads?api-version=2025-05-01"
    );
    assert_eq!(
        requests[2].url,
        "https://res.services.ai.azure.com/threads/thread_1/runs?api-version=2025-05-01"
    );
    assert_eq!(
        requests[2].body.as_deref(),
        Some(r#"{"assistant_id":"my-agent"}"#)
    );
    assert_eq!(
        requests[3].url,
        "https://res.services.ai.azure.com/threads/thread_1/runs/run_1?api-version=2025-05-01"
    );
    let headers = &requests[0].headers;
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value == "Bearer entra-token")
    );
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "x-team" && value == "alpha")
    );
    {
        let seen = resolver.seen.lock().expect("the resolver calls");
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].get("azure_scope"),
            Some(&PromptJson::Text(String::from(
                "https://ai.azure.com/.default"
            )))
        );
    }

    // an async stream call is handed to B562 to open, without running the polling flow here
    let stream_script = Arc::new(Scripted::new(Vec::new()));
    let stream_factory = AgentFactory {
        script: Arc::clone(&stream_script),
    };
    let call = agents::Call {
        model: "agents/my-agent",
        messages: &messages,
        optional_params: &optional,
        litellm_params: &params,
        headers: None,
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        acompletion: true,
        stream: true,
        reply: agent_reply(),
    };
    let outcome = agents::Dispatch {
        call,
        globals: env.globals(),
        deps: agents::Deps {
            factory: &stream_factory,
            sleeper: &clock,
            counter: &tally,
            resolver: &resolver,
        },
    }
    .run()
    .await
    .expect("the agent stream dispatch runs");
    let agents::Outcome::Stream(request) = outcome else {
        panic!("the async stream call builds a stream request");
    };
    assert_eq!(
        request.url,
        "https://res.services.ai.azure.com/threads/runs?api-version=2025-05-01"
    );
    assert_eq!(request.payload.assistant_id, "my-agent");
    assert!(stream_script.requests().is_empty());
}

async fn agents_missing_base() {
    let script = Arc::new(Scripted::new(Vec::new()));
    let factory = AgentFactory { script };
    let clock = Clock::default();
    let tally = AgentTally;
    let resolver = Resolver {
        token: None,
        seen: Mutex::new(Vec::new()),
    };
    let messages = agent_messages();
    let optional = PromptObject::new();
    let params = PromptObject::new();
    let env = Env::default();
    let call = agents::Call {
        model: "agents/my-agent",
        messages: &messages,
        optional_params: &optional,
        litellm_params: &params,
        headers: None,
        api_base: None,
        api_key: None,
        acompletion: false,
        stream: false,
        reply: agent_reply(),
    };
    let dispatch = agents::Dispatch {
        call,
        globals: env.globals(),
        deps: agents::Deps {
            factory: &factory,
            sleeper: &clock,
            counter: &tally,
            resolver: &resolver,
        },
    };
    let error = dispatch.run().await.expect_err("no base is a failure");
    assert_eq!(error.to_string(), route::AGENTS_BASE_MESSAGE);

    // a supplied map wins over the global map
    let env = Env::new().headers(BTreeMap::from([(
        String::from("x-global"),
        String::from("g"),
    )]));
    let supplied = BTreeMap::from([(String::from("x-supplied"), String::from("s"))]);
    let call = agents::Call {
        model: "agents/my-agent",
        messages: &messages,
        optional_params: &optional,
        litellm_params: &params,
        headers: Some(&supplied),
        api_base: Some("https://res.services.ai.azure.com"),
        api_key: None,
        acompletion: false,
        stream: false,
        reply: agent_reply(),
    };
    let resolved = agents::resolve(&call, &env.globals()).expect("the base resolves");
    assert_eq!(resolved.api_base, "https://res.services.ai.azure.com");
    assert!(resolved.headers.contains_key("x-supplied"));
    assert!(!resolved.headers.contains_key("x-global"));
}

async fn claude_missing_base() {
    let message: PromptObject =
        serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("a message");
    let messages = [message];
    let params = PromptObject::new();
    let key = SecretString::from(KEY);
    let env = Env::default();
    let mut call = claude_call(&messages, &params, &key, false, false);
    call.api_base = None;
    let error = claude::prepare(&call, &env.globals())
        .err()
        .expect("no base is a failure");
    assert_eq!(error.to_string(), route::CLAUDE_BASE_MESSAGE);

    // a global base is a fallback, and a global header map never reaches this branch
    let env = Env::new()
        .global_base("https://res.services.ai.azure.com")
        .headers(BTreeMap::from([(
            String::from("x-global"),
            String::from("g"),
        )]));
    let globals = env.globals();
    let prepared = claude::prepare(&call, &globals).expect("the global base resolves");
    let request = claude::request(&prepared, &call, &globals);
    assert_eq!(
        request.api_base,
        "https://res.services.ai.azure.com/anthropic/v1/messages"
    );
    assert!(request.caller_headers.is_empty());
}

fn claude_url_rewrite() {
    let rows = [
        ("https://x.test", "https://x.test/anthropic/v1/messages"),
        ("https://x.test/", "https://x.test/anthropic/v1/messages"),
        (
            "https://x.test/anthropic",
            "https://x.test/anthropic/v1/messages",
        ),
        (
            "https://x.test/anthropic/v1/messages",
            "https://x.test/anthropic/v1/messages",
        ),
        (
            "https://x.test/anthropic/v1/messages/",
            "https://x.test/anthropic/v1/messages",
        ),
        (
            "https://x.test/anthropic/other/route",
            "https://x.test/anthropic/v1/messages",
        ),
        ("", ""),
    ];
    for (base, expected) in rows {
        assert_eq!(claude::endpoint(base), expected, "base {base}");
    }
}

fn command_r_case() {
    let request = support::request(
        r#"{"model":"command-r-plus","messages":[{"role":"tool","content":"not json"},{"role":"user","content":"next"}]}"#,
    );
    let deployment = support::deployment(BASE, Some(KEY));
    let env = Env::default();
    let call = chat_call(&request, &deployment, "command-r-plus", false);
    let plan = chat::prepare::prepare(&call, &env.globals()).expect("the command-r plan");
    assert_eq!(plan.messages.len(), 2);
    assert_eq!(plan.messages[0].role, "tool");
    assert_eq!(
        plan.messages[0].content,
        Some(Content::Text(String::from(r#"{"result":"not json"}"#)))
    );
    assert_eq!(
        plan.messages[1].content,
        Some(Content::Text(String::from("next")))
    );

    // Command-R keeps its messages untouched, since LiteLLM's substring check is case-sensitive
    let request = support::request(
        r#"{"model":"Command-R","messages":[{"role":"tool","content":"not json"}]}"#,
    );
    let call = chat_call(&request, &deployment, "Command-R", false);
    let plan = chat::prepare::prepare(&call, &env.globals()).expect("the plain plan");
    assert_eq!(
        plan.messages[0].content,
        Some(Content::Text(String::from("not json")))
    );

    // a supplied extra_headers map, even an empty one, replaces the optional map's own entry
    let request = support::request(
        r#"{"model":"mistral-large","messages":[{"role":"user","content":"hi"}],"extra_headers":{"x-given":"1"}}"#,
    );
    let call = chat_call(&request, &deployment, "mistral-large", false);
    let plan = chat::prepare::prepare(&call, &env.globals()).expect("the kept plan");
    let PreparedRequest::Chat(prepared) = &plan.prepared else {
        panic!("the chat call prepares a chat request");
    };
    assert_eq!(
        prepared
            .http
            .headers
            .get("x-given")
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );
    let empty = BTreeMap::new();
    let call = chat::Call {
        extra_headers: Some(&empty),
        ..chat_call(&request, &deployment, "mistral-large", false)
    };
    let plan = chat::prepare::prepare(&call, &env.globals()).expect("the overwritten plan");
    let PreparedRequest::Chat(prepared) = &plan.prepared else {
        panic!("the chat call prepares a chat request");
    };
    assert!(prepared.http.headers.get("x-given").is_none());
}

async fn http_case() {
    let (listener, api_base) = loopback().await;
    let deployment = support::deployment(&api_base, Some(KEY));
    let catalog = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    let request = OperationRequest::Chat(Box::new(ChatOperation {
        model: String::from("mistral-large"),
        // the optional parameters reach the body, and api_version becomes the URL query alone
        request: support::request(
            r#"{"model":"mistral-large","messages":[{"role":"user","content":"hi"}],"max_tokens":7,"api_version":"2024-02-01"}"#,
        ),
        control: ControlOptions::default(),
    }));
    let prepared = AzureAi
        .prepare(&request, &deployment, &catalog)
        .expect("the adapter prepares the chat call");
    let PreparedRequest::Chat(prepared_chat) = &prepared else {
        panic!("the chat call prepares a chat request");
    };
    let expected = prepared_chat.http.body.clone();
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{REPLY}",
        REPLY.len()
    );
    let client = proxied_client();
    let (upstream, sent) = tokio::join!(
        AzureAi.execute(&client, &prepared, &deployment),
        serve_once(listener, &expected, &reply)
    );
    let decoded = AzureAi
        .decode(&prepared, upstream.expect("the loopback dispatch runs"))
        .expect("the adapter decodes the reply");
    let OperationResult::Chat(ChatCompletion::OpenAi(completion)) = decoded else {
        panic!("the loopback dispatch returns a chat reply");
    };
    assert_eq!(
        completion.choices[0].message.content.as_deref(),
        Some("hello")
    );
    assert!(sent.starts_with("POST /models/chat/completions?api-version=2024-02-01 HTTP/1.1"));
    let lower = sent.to_lowercase();
    assert!(lower.contains("authorization: bearer sk-foundry-fixture"));
    assert!(lower.contains("content-type: application/json"));
    let body = sent.split("\r\n\r\n").nth(1).expect("the request body");
    let body: JsonValue = serde_json::from_str(body).expect("the request body");
    assert_eq!(
        field(&body, "model"),
        &JsonValue::Text(String::from("mistral-large"))
    );
    let max_tokens = JsonValue::Number(serde_json::Number::from(7_u8));
    assert_eq!(field(&body, "max_tokens"), &max_tokens);
    let JsonValue::Array(messages) = field(&body, "messages") else {
        panic!("the sent messages are an array");
    };
    assert_eq!(
        field(messages.first().expect("a message"), "content"),
        &JsonValue::Text(String::from("hi"))
    );

    // the adapter refuses the operation kinds it does not run
    let error = AzureAi
        .prepare(
            &OperationRequest::AzureAi(AzureAiRequest),
            &deployment,
            &catalog,
        )
        .expect_err("the other kind is refused");
    assert_eq!(
        error.to_string(),
        "betterlite does not run the provider operation through the azure_ai provider"
    );
}

async fn post_call_case() {
    let env = Env::new().headers(BTreeMap::from([(
        String::from("x-logged"),
        String::from("1"),
    )]));
    let deployment = support::deployment(BASE, Some(KEY));

    // a normal non-stream success logs nothing
    let request = support::simple_request("mistral-large");
    let call = chat::Call {
        headers: env.headers.as_ref(),
        ..chat_call(&request, &deployment, "mistral-large", false)
    };
    let mut logger = RecordingLogger::default();
    chat::Dispatch {
        call,
        globals: env.globals(),
        answer: support::supplied(200, REPLY),
    }
    .run(&mut logger)
    .await
    .expect("the chat dispatch runs");
    assert!(logger.calls.is_empty());

    // a stream success logs the stream report with the resolved headers, through the real client
    let (listener, api_base) = loopback().await;
    let stream_deployment = support::deployment(&api_base, Some(KEY));
    let call = chat::Call {
        headers: env.headers.as_ref(),
        ..chat_call(&request, &stream_deployment, "mistral-large", true)
    };
    let plan = chat::prepare::prepare(&call, &env.globals()).expect("the stream plan");
    let PreparedRequest::Stream(prepared) = &plan.prepared else {
        panic!("the stream call prepares a stream request");
    };
    let expected = prepared.http.body.clone();
    let reply = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\ndata: {\"id\":\"chatcmpl-foundry\"}\n\n";
    let client = proxied_client();
    let mut logger = RecordingLogger::default();
    let (result, _sent) = tokio::join!(
        async {
            chat::Dispatch {
                call,
                globals: env.globals(),
                answer: Answer::Client(&client),
            }
            .run(&mut logger)
            .await
        },
        serve_once(listener, &expected, reply)
    );
    result.expect("the stream dispatch runs");
    assert_eq!(logger.calls.len(), 1);
    assert_eq!(
        logger.calls[0].original_response,
        Json::Object(IndexMap::from([(String::from("stream"), Json::Bool(true))]))
    );
    let headers = logger.calls[0].headers.as_ref().expect("the headers");
    assert_eq!(
        headers.get("x-logged"),
        Some(&Json::Text(String::from("1")))
    );

    // an execution failure logs the error text plus the headers, then rethrows
    let call = chat::Call {
        headers: env.headers.as_ref(),
        ..chat_call(&request, &deployment, "mistral-large", false)
    };
    let mut logger = RecordingLogger::default();
    let error = chat::Dispatch {
        call,
        globals: env.globals(),
        answer: support::supplied(500, r#"{"error":"boom"}"#),
    }
    .run(&mut logger)
    .await
    .expect_err("a 500 is a failure");
    assert!(matches!(error, OperationError::Provider { .. }));
    assert_eq!(logger.calls.len(), 1);
    let Json::Text(logged) = &logger.calls[0].original_response else {
        panic!("the failure log carries the error text");
    };
    assert_eq!(logged, &error.to_string());
    assert!(logger.calls[0].headers.is_some());

    // a preparation failure logs nothing
    let claude_request = support::simple_request("claude-sonnet");
    let call = chat::Call {
        headers: env.headers.as_ref(),
        ..chat_call(&claude_request, &deployment, "claude-sonnet", false)
    };
    let mut logger = RecordingLogger::default();
    assert!(
        chat::Dispatch {
            call,
            globals: env.globals(),
            answer: support::supplied(200, REPLY),
        }
        .run(&mut logger)
        .await
        .is_err()
    );
    assert!(logger.calls.is_empty());
}
