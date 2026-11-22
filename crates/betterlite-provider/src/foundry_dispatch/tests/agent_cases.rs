use std::sync::{Arc, Mutex};

use betterlite_foundry::agent_results::reply::AgentReply;
use betterlite_foundry::agent_runs::transport::AgentRunMethod;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};

use super::support::{self, Env, HOST};
use crate::foundry_dispatch::agents;
use crate::foundry_dispatch::fixtures::agents::{Clock, Factory, Resolver, Scripted, Step, Tally};
use crate::foundry_dispatch::route;

const THREADS: &str = "https://res.services.ai.azure.com/threads";

// The scripted transport, clock and resolver one agent dispatch runs against
struct Rig {
    factory: Factory,
    clock: Clock,
    resolver: Resolver,
    messages: Vec<PromptObject>,
    optional: PromptObject,
    params: PromptObject,
}

impl Rig {
    fn new(steps: Vec<Step>, token: Option<&str>) -> Self {
        Self {
            factory: Factory {
                script: Arc::new(Scripted::new(steps)),
            },
            clock: Clock::default(),
            resolver: Resolver {
                token: token.map(str::to_owned),
                seen: Mutex::new(Vec::new()),
            },
            messages: vec![
                serde_json::from_str(r#"{"role":"user","content":"hi"}"#).expect("a message"),
            ],
            optional: PromptObject::new(),
            params: PromptObject::new(),
        }
    }

    // A streamed call is also an async one, as the stream handoff needs
    fn call<'rig>(
        &'rig self,
        api_base: Option<&'rig str>,
        headers: Option<&'rig Headers>,
        streamed: bool,
    ) -> agents::Call<'rig> {
        agents::Call {
            model: "agents/my-agent",
            messages: &self.messages,
            optional_params: &self.optional,
            litellm_params: &self.params,
            headers,
            api_base,
            api_key: None,
            acompletion: streamed,
            stream: streamed,
            reply: AgentReply::default(),
        }
    }

    fn run(
        &self,
        call: agents::Call<'_>,
        env: &Env,
    ) -> Result<agents::Outcome, agents::DispatchError> {
        support::block_on(
            agents::Dispatch {
                call,
                globals: env.globals(),
                deps: agents::Deps {
                    factory: &self.factory,
                    sleeper: &self.clock,
                    counter: &Tally,
                    resolver: &self.resolver,
                },
            }
            .run(),
        )
    }
}

fn full_script() -> Vec<Step> {
    vec![
        Step::new(201, r#"{"id":"thread_1"}"#),
        Step::new(201, r#"{"id":"message_1"}"#),
        Step::new(201, r#"{"id":"run_1"}"#),
        Step::new(200, r#"{"status":"in_progress"}"#),
        Step::new(200, r#"{"status":"completed"}"#),
        Step::new(
            200,
            r#"{"data":[{"role":"assistant","content":[{"type":"text","text":{"value":"hello"}}]}]}"#,
        ),
    ]
}

pub(super) fn agent_case() {
    polling_reply();
    stream_is_handed_off();
}

fn polling_reply() {
    let rig = Rig::new(full_script(), Some("entra-token"));
    let env = Env::with_header("x-team", "alpha");
    let outcome = rig
        .run(rig.call(Some(HOST), None, false), &env)
        .expect("the agent dispatch runs");
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
    check_flow(&rig);
    check_credentials(&rig);
}

fn check_flow(rig: &Rig) {
    let requests = rig.factory.script.requests();
    assert_eq!(requests.len(), 6);
    // the poll that finds the run unfinished waits once in the supplied clock
    assert_eq!(rig.clock.waits.lock().expect("the poll waits").len(), 1);
    assert_eq!(requests[0].method, AgentRunMethod::Post);
    assert_eq!(requests[0].url, format!("{THREADS}?api-version=2025-05-01"));
    assert_eq!(
        requests[2].url,
        format!("{THREADS}/thread_1/runs?api-version=2025-05-01")
    );
    assert_eq!(
        requests[2].body.as_deref(),
        Some(r#"{"assistant_id":"my-agent"}"#)
    );
    assert_eq!(
        requests[3].url,
        format!("{THREADS}/thread_1/runs/run_1?api-version=2025-05-01")
    );
}

fn check_credentials(rig: &Rig) {
    let requests = rig.factory.script.requests();
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
    let seen = rig.resolver.seen.lock().expect("the resolver calls");
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].get("azure_scope"),
        Some(&PromptJson::Text(String::from(
            "https://ai.azure.com/.default"
        )))
    );
}

// an async stream call is handed to B562 to open, without running the polling flow here
fn stream_is_handed_off() {
    let rig = Rig::new(Vec::new(), Some("entra-token"));
    let env = Env::with_header("x-team", "alpha");
    let outcome = rig
        .run(rig.call(Some(HOST), None, true), &env)
        .expect("the agent stream dispatch runs");
    let agents::Outcome::Stream(request) = outcome else {
        panic!("the async stream call builds a stream request");
    };
    assert_eq!(
        request.url,
        format!("{THREADS}/runs?api-version=2025-05-01")
    );
    assert_eq!(request.payload.assistant_id, "my-agent");
    assert!(rig.factory.script.requests().is_empty());
}

pub(super) fn agents_missing_base() {
    agents_without_base_fails();
    supplied_headers_win();
}

fn agents_without_base_fails() {
    let rig = Rig::new(Vec::new(), None);
    let Err(error) = rig.run(rig.call(None, None, false), &Env::default()) else {
        panic!("no base is a failure");
    };
    assert_eq!(error.to_string(), route::AGENTS_BASE_MESSAGE);
}

// a supplied map wins over the global map
fn supplied_headers_win() {
    let rig = Rig::new(Vec::new(), None);
    let env = Env::with_header("x-global", "g");
    let supplied = Headers::from([(String::from("x-supplied"), String::from("s"))]);
    let call = rig.call(Some(HOST), Some(&supplied), false);
    let authorized = agents::resolve(&call, &env.globals()).expect("the base resolves");
    assert_eq!(authorized.api_base, HOST);
    assert!(authorized.headers.contains_key("x-supplied"));
    assert!(!authorized.headers.contains_key("x-global"));
}
