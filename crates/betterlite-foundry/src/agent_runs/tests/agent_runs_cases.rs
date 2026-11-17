#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;

use crate::agent_results::reply::{AgentReply, Usage};
use crate::agent_runs::decode::{RUN_CANCELLED, RUN_EXPIRED, RUN_FAILED};
use crate::agent_runs::dispatch::{Route, dispatch};
use crate::agent_runs::error::AgentRunError;
use crate::agent_runs::flow::{CompletionCall, MAX_POLL_ATTEMPTS, POLL_INTERVAL, completion};
use crate::agent_runs::transport::{AgentRunMethod, AgentRunMode, AgentRunTransport};

use super::support::{
    BASE, CallFixture, FactoryCall, FailingResolver, FixedCounter, MODEL, Outcome, RecordedRequest,
    RecordingResolver, RecordingSleeper, Run, SECRET, Step, UnusedFactory, block_on, execute,
    failed_step, finish, messages, object, prepared, repeated, shown, status_step, step, transport,
};

const MISSING_TOKEN: &str = "api_key (Azure AD token) is required for Azure Foundry Agents. Either pass \
     api_key directly, or set AZURE_TENANT_ID, AZURE_CLIENT_ID, and AZURE_CLIENT_SECRET environment \
     variables for Service Principal auth. Manual token: az account get-access-token --resource \
     'https://ai.azure.com'";

fn url(path: &str) -> String {
    format!("{BASE}{path}?api-version=2025-05-01")
}

fn reply_of(outcome: &Outcome) -> &AgentReply {
    match &outcome.result {
        Ok(reply) => reply,
        Err(error) => panic!("expected a reply, got {error}"),
    }
}

fn failure_of(outcome: &Outcome) -> &AgentRunError {
    match &outcome.result {
        Err(error) => error,
        Ok(reply) => panic!("expected a failure, got {reply:?}"),
    }
}

fn assert_request(
    request: &RecordedRequest,
    method: AgentRunMethod,
    expected: &str,
    body: Option<&Object>,
) {
    assert_eq!(request.method, method);
    assert_eq!(request.url, expected);
    assert_eq!(
        request.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    match (body, &request.body) {
        (None, None) => {}
        (Some(wanted), Some(bytes)) => {
            let actual: Object = serde_json::from_slice(bytes).expect("a JSON body");
            assert_eq!(&actual, wanted);
        }
        (wanted, actual) => panic!("expected {wanted:?}, got {actual:?}"),
    }
}

fn assert_auth(request: &RecordedRequest) {
    assert_eq!(
        request.headers.get("Authorization").map(String::as_str),
        Some(format!("Bearer {SECRET}").as_str())
    );
}

fn hello_messages() -> Step {
    step(
        200,
        r#"{"data": [{"role": "assistant", "content": [{"type": "text", "text": {"value": "hello", "annotations": []}}]}]}"#,
    )
}

fn completed_steps() -> Vec<Step> {
    vec![
        step(201, r#"{"id": "thread-1"}"#),
        step(201, r#"{"id": "run-1"}"#),
        status_step("completed"),
        hello_messages(),
    ]
}

fn new_thread_run_flow() {
    sync_new_thread();
    async_new_thread();
    falsey_thread_ids_create_a_thread();
}

fn sync_new_thread() {
    let mut case = Run::sync(vec![
        step(201, r#"{"id": "thread-1", "object": "thread"}"#),
        step(200, r#"{"id": "msg-1"}"#),
        step(201, r#"{"id": "run-1", "status": "queued"}"#),
        status_step("completed"),
        hello_messages(),
    ]);
    case.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let outcome = execute(&case);
    assert!(outcome.factory_calls.is_empty());
    assert!(outcome.sleeps.is_empty());
    assert_new_thread_requests(&outcome);
    assert_new_thread_reply(reply_of(&outcome));
}

fn assert_new_thread_requests(outcome: &Outcome) {
    let state = outcome.transport.state();
    let requests = state.requests();
    assert_eq!(requests.len(), 5);
    assert_request(&requests[0], AgentRunMethod::Post, &url("/threads"), None);
    assert_auth(&requests[0]);
    assert_request(
        &requests[1],
        AgentRunMethod::Post,
        &url("/threads/thread-1/messages"),
        Some(&object(r#"{"role": "user", "content": "hi"}"#)),
    );
    assert_request(
        &requests[2],
        AgentRunMethod::Post,
        &url("/threads/thread-1/runs"),
        Some(&object(r#"{"assistant_id": "asst_1"}"#)),
    );
    assert_request(
        &requests[3],
        AgentRunMethod::Get,
        &url("/threads/thread-1/runs/run-1"),
        None,
    );
    assert_request(
        &requests[4],
        AgentRunMethod::Get,
        &url("/threads/thread-1/messages"),
        None,
    );
    assert_eq!(state.remaining(), 0);
}

fn assert_new_thread_reply(reply: &AgentReply) {
    assert_eq!(reply.model.as_deref(), Some(MODEL));
    assert_eq!(reply.choices.len(), 1);
    assert_eq!(reply.choices[0].message.role, "assistant");
    assert_eq!(reply.choices[0].message.content, "hello");
    assert_eq!(reply.choices[0].message.annotations, None);
    assert_eq!(reply.choices[0].finish_reason, "stop");
    assert_eq!(
        reply.usage,
        Some(Usage {
            prompt_tokens: 1,
            completion_tokens: 5,
            total_tokens: 6,
        })
    );
    let thread = reply
        .hidden_params
        .as_ref()
        .and_then(|params| params.get("thread_id"))
        .and_then(Json::as_str);
    assert_eq!(thread, Some("thread-1"));
}

fn async_new_thread() {
    let mut created = Run::sync(vec![
        step(200, r#"{"id": "thread-2"}"#),
        step(201, r#"{"id": "msg-2"}"#),
        step(200, r#"{"id": "run-2"}"#),
        status_step("completed"),
        step(
            200,
            r#"{"data": [{"role": "assistant", "content": [{"type": "text", "text": {"value": "async"}}]}]}"#,
        ),
    ]);
    created.mode = AgentRunMode::Async;
    created.supplied = false;
    created.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let outcome = execute(&created);
    assert_eq!(
        outcome.factory_calls,
        vec![FactoryCall {
            kind: "async",
            ssl_verify: Json::Null,
        }]
    );
    let state = outcome.transport.state();
    assert_eq!(state.requests().len(), 5);
    assert_eq!(state.requests()[0].url, url("/threads"));
    assert_eq!(state.requests()[3].url, url("/threads/thread-2/runs/run-2"));
    assert_eq!(state.remaining(), 0);
    drop(state);
    assert_eq!(reply_of(&outcome).choices[0].message.content, "async");
}

fn falsey_thread_ids_create_a_thread() {
    for thread_id in [r#"{"thread_id": null}"#, r#"{"thread_id": ""}"#] {
        let mut falsey = Run::sync(vec![
            step(201, r#"{"id": "thread-3"}"#),
            step(201, r#"{"id": "run-3"}"#),
            status_step("completed"),
            hello_messages(),
        ]);
        falsey.optional_params = object(thread_id);
        let outcome = execute(&falsey);
        let state = outcome.transport.state();
        assert_eq!(state.requests().len(), 4);
        assert_request(
            &state.requests()[0],
            AgentRunMethod::Post,
            &url("/threads"),
            None,
        );
        assert_request(
            &state.requests()[1],
            AgentRunMethod::Post,
            &url("/threads/thread-3/runs"),
            Some(&object(r#"{"assistant_id": "asst_1"}"#)),
        );
        assert_eq!(state.remaining(), 0);
        drop(state);
        assert_eq!(reply_of(&outcome).choices[0].message.content, "hello");
    }
}

fn existing_thread_message_flow() {
    let outcome = existing_thread_outcome();
    assert_thread_message_posts(&outcome);
    assert_thread_run_and_reads(&outcome);
    assert_eq!(reply_of(&outcome).choices[0].message.content, "done");
    run_body_keeps_instruction_values();
}

fn existing_thread_outcome() -> Outcome {
    let mut case = Run::sync(vec![
        step(200, r#"{"id": "msg-1"}"#),
        step(201, r#"{"id": "msg-2"}"#),
        step(200, r#"{"id": "msg-3"}"#),
        step(201, r#"{"id": "msg-4"}"#),
        step(200, r#"{"id": "msg-5"}"#),
        step(201, r#"{"id": "run-1"}"#),
        status_step("completed"),
        step(
            200,
            r#"{"data": [{"role": "assistant", "content": [{"type": "text", "text": {"value": "done"}}]}]}"#,
        ),
    ]);
    case.optional_params = object(r#"{"thread_id": "thread-9", "instructions": ""}"#);
    case.messages = messages(
        r#"[
            {"role": "user", "content": "one"},
            {"role": "system", "content": "two"},
            {"role": "assistant", "content": "skip"},
            {"role": "tool", "content": "skip"},
            {"role": "user"},
            {"role": "user", "content": null},
            {"content": "no role"},
            {"role": "user", "content": [{"type": "text", "text": {"value": "block"}}]}
        ]"#,
    );
    execute(&case)
}

fn assert_thread_message_posts(outcome: &Outcome) {
    let state = outcome.transport.state();
    let requests = state.requests();
    assert_eq!(requests.len(), 8);
    let message_url = url("/threads/thread-9/messages");
    let expected = [
        r#"{"role": "user", "content": "one"}"#,
        r#"{"role": "user", "content": "two"}"#,
        r#"{"role": "user", "content": ""}"#,
        r#"{"role": "user", "content": null}"#,
        r#"{"role": "user", "content": [{"type": "text", "text": {"value": "block"}}]}"#,
    ];
    for (index, body) in expected.iter().enumerate() {
        assert_request(
            &requests[index],
            AgentRunMethod::Post,
            &message_url,
            Some(&object(body)),
        );
    }
    assert_eq!(state.remaining(), 0);
}

fn assert_thread_run_and_reads(outcome: &Outcome) {
    let state = outcome.transport.state();
    let requests = state.requests();
    assert!(requests.len() > 7);
    assert_request(
        &requests[5],
        AgentRunMethod::Post,
        &url("/threads/thread-9/runs"),
        Some(&object(r#"{"assistant_id": "asst_1", "instructions": ""}"#)),
    );
    assert_request(
        &requests[6],
        AgentRunMethod::Get,
        &url("/threads/thread-9/runs/run-1"),
        None,
    );
    assert_request(
        &requests[7],
        AgentRunMethod::Get,
        &url("/threads/thread-9/messages"),
        None,
    );
    assert_eq!(state.remaining(), 0);
}

fn run_body_keeps_instruction_values() {
    let variants = [
        (
            r#"{"thread_id": "thread-9", "instructions": null}"#,
            r#"{"assistant_id": "asst_1", "instructions": null}"#,
        ),
        (
            r#"{"thread_id": "thread-9", "instructions": []}"#,
            r#"{"assistant_id": "asst_1", "instructions": []}"#,
        ),
        (
            r#"{"thread_id": "thread-9"}"#,
            r#"{"assistant_id": "asst_1"}"#,
        ),
    ];
    for (optional_params, run_body) in variants {
        let mut variant = Run::sync(vec![
            step(200, r#"{"id": "run-1"}"#),
            status_step("completed"),
            hello_messages(),
        ]);
        variant.optional_params = object(optional_params);
        let outcome = execute(&variant);
        let state = outcome.transport.state();
        assert_request(
            &state.requests()[0],
            AgentRunMethod::Post,
            &url("/threads/thread-9/runs"),
            Some(&object(run_body)),
        );
    }
}

fn failed_cancelled_expired_run() {
    terminal_statuses_fail();
    missing_messages_use_unknown_error();
    other_statuses_keep_polling();
}

fn terminal_statuses_fail() {
    for (status, expected) in [
        (RUN_FAILED, "Run failed: boom"),
        (RUN_CANCELLED, "Run cancelled: boom"),
        (RUN_EXPIRED, "Run expired: boom"),
    ] {
        let mut case = Run::sync(vec![
            step(201, r#"{"id": "run-1"}"#),
            failed_step(status, "boom"),
        ]);
        case.optional_params = object(r#"{"thread_id": "thread-9"}"#);
        let outcome = execute(&case);
        let error = failure_of(&outcome);
        assert_eq!(error.status(), Some(500));
        assert_eq!(error.to_string(), expected);
        assert!(outcome.sleeps.is_empty());
        let state = outcome.transport.state();
        assert_eq!(state.requests().len(), 2);
        assert_eq!(state.remaining(), 0);
    }
}

fn missing_messages_use_unknown_error() {
    let unknown = [
        status_step(RUN_FAILED),
        step(200, r#"{"status": "failed", "last_error": {}}"#),
        step(
            200,
            r#"{"status": "failed", "last_error": {"message": null}}"#,
        ),
        step(200, r#"{"status": "failed", "last_error": null}"#),
    ];
    for body in unknown {
        let mut case = Run::sync(vec![step(201, r#"{"id": "run-1"}"#), body]);
        case.optional_params = object(r#"{"thread_id": "thread-9"}"#);
        let outcome = execute(&case);
        let error = failure_of(&outcome);
        assert_eq!(error.status(), Some(500));
        assert_eq!(error.to_string(), "Run failed: Unknown error");
        assert!(outcome.sleeps.is_empty());
    }
}

fn other_statuses_keep_polling() {
    let mut polling = Run::sync(vec![
        step(201, r#"{"id": "run-1"}"#),
        status_step("in_progress"),
        step(200, r#"{"status": null}"#),
        step(200, "{}"),
        status_step("queued"),
        status_step("completed"),
        hello_messages(),
    ]);
    polling.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let outcome = execute(&polling);
    assert_eq!(outcome.sleeps, vec![POLL_INTERVAL; 4]);
    let state = outcome.transport.state();
    assert_eq!(state.requests().len(), 7);
    assert_eq!(state.remaining(), 0);
    drop(state);
    assert_eq!(reply_of(&outcome).choices[0].message.content, "hello");
}

fn nonexpected_status() {
    create_and_message_failures();
    run_and_status_failures();
    messages_and_binary_failures();
    decode_failures_are_typed();
}

fn decode_failures_are_typed() {
    let created = [
        (
            r#""thread-1""#,
            "the agent response is not a JSON object",
            1,
        ),
        (r#"{"id": "#, "the agent response is not valid JSON", 1),
        (r#"{"id": 7}"#, "the agent response is not valid JSON", 1),
    ];
    for (body, expected, requests) in created {
        let outcome = execute(&Run::sync(vec![step(201, body)]));
        assert_decode_failure(&outcome, expected, requests);
    }
    let mut polled = Run::sync(vec![
        step(201, r#"{"id": "run-1"}"#),
        step(200, r#"{"status": 3}"#),
    ]);
    polled.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    assert_decode_failure(&execute(&polled), "the agent response is not valid JSON", 2);
}

fn assert_decode_failure(outcome: &Outcome, expected: &str, requests: usize) {
    let error = failure_of(outcome);
    assert_eq!(error.status(), None);
    assert_eq!(error.to_string(), expected);
    assert_eq!(outcome.transport.state().requests().len(), requests);
}

fn assert_upstream_failure(
    case: &Run,
    expected_status: u16,
    expected_text: &str,
    expected_requests: usize,
) {
    let outcome = execute(case);
    let error = failure_of(&outcome);
    assert_eq!(error.status(), Some(expected_status));
    assert_eq!(error.to_string(), expected_text);
    assert!(outcome.sleeps.is_empty());
    assert_eq!(
        outcome.transport.state().requests().len(),
        expected_requests
    );
}

fn create_and_message_failures() {
    let mut create = Run::sync(vec![step(500, "thread broke")]);
    create.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    assert_upstream_failure(&create, 500, "Failed to create thread: thread broke", 1);

    let mut message = Run::sync(vec![step(400, "bad message")]);
    message.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    message.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    assert_upstream_failure(&message, 400, "Failed to add message: bad message", 1);
}

fn run_and_status_failures() {
    let mut run = Run::sync(vec![step(429, "slow down")]);
    run.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    assert_upstream_failure(&run, 429, "Failed to create run: slow down", 1);

    let mut status = Run::sync(vec![
        step(201, r#"{"id": "run-1"}"#),
        step(503, "status down"),
    ]);
    status.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    assert_upstream_failure(&status, 503, "Failed to get run status: status down", 2);
}

fn messages_and_binary_failures() {
    let mut list = Run::sync(vec![
        step(201, r#"{"id": "run-1"}"#),
        status_step("completed"),
        step(500, "messages broke"),
    ]);
    list.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    assert_upstream_failure(&list, 500, "Failed to get messages: messages broke", 3);

    let mut binary = Run::sync(vec![Step {
        status: 500,
        body: vec![0xff, 0xfe],
    }]);
    binary.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    assert_upstream_failure(
        &binary,
        500,
        "Failed to create thread: the response body is not UTF-8",
        1,
    );
}

fn sixty_polls_408() {
    timeout_after_sixty_polls();
    completed_status_ends_at_once();
    last_attempt_can_complete();
}

fn timeout_after_sixty_polls() {
    let mut steps = vec![step(201, r#"{"id": "run-1"}"#)];
    steps.extend(repeated(MAX_POLL_ATTEMPTS, &status_step("in_progress")));
    let mut timed_out = Run::sync(steps);
    timed_out.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let outcome = execute(&timed_out);
    let error = failure_of(&outcome);
    assert_eq!(error.status(), Some(408));
    assert_eq!(error.to_string(), "Run timed out waiting for completion");
    assert_eq!(outcome.sleeps.len(), MAX_POLL_ATTEMPTS);
    assert!(outcome.sleeps.iter().all(|wait| *wait == POLL_INTERVAL));
    let state = outcome.transport.state();
    assert_eq!(state.requests().len(), MAX_POLL_ATTEMPTS.saturating_add(1));
    assert_eq!(state.remaining(), 0);
}

fn completed_status_ends_at_once() {
    let mut immediate = Run::sync(vec![
        step(201, r#"{"id": "run-1"}"#),
        status_step("completed"),
        hello_messages(),
    ]);
    immediate.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let outcome = execute(&immediate);
    assert!(outcome.sleeps.is_empty());
    let state = outcome.transport.state();
    assert_eq!(state.requests().len(), 3);
    assert_eq!(state.requests()[1].url, url("/threads/thread-9/runs/run-1"));
    assert_eq!(state.requests()[2].url, url("/threads/thread-9/messages"));
}

fn last_attempt_can_complete() {
    let mut steps = vec![step(201, r#"{"id": "run-1"}"#)];
    steps.extend(repeated(
        MAX_POLL_ATTEMPTS.saturating_sub(1),
        &status_step("in_progress"),
    ));
    steps.push(status_step("completed"));
    steps.push(hello_messages());
    let mut last = Run::sync(steps);
    last.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let outcome = execute(&last);
    assert_eq!(outcome.sleeps.len(), MAX_POLL_ATTEMPTS.saturating_sub(1));
    assert_eq!(reply_of(&outcome).choices[0].message.content, "hello");
}

fn agent_entra_forced_scope() {
    forced_scope_resolves_token();
    empty_key_reuses_the_token();
    stream_payload_carries_scope_and_ssl();
}

fn forced_scope_resolves_token() {
    let resolver = RecordingResolver::new(Some("resolved-token"));
    let mut fixture = CallFixture::new(BASE);
    fixture.litellm_params = object(
        r#"{"tenant_id": "tenant-a", "client_id": "client-a", "client_secret": "secret-a",
            "azure_scope": "https://wrong.example/.default", "ssl_verify": false, "extra": "kept"}"#,
    );
    let route = dispatch(fixture.call(), &resolver).expect("a route");
    let params = resolver.params();
    assert_eq!(params.len(), 1);
    assert_forced_scope(&params[0]);
    assert_eq!(
        fixture
            .litellm_params
            .get("azure_scope")
            .and_then(Json::as_str),
        Some("https://wrong.example/.default")
    );
    match route {
        Route::Completion(reused) => {
            assert_eq!(reused.mode, AgentRunMode::Sync);
            assert_eq!(reused.ssl_verify, Json::Bool(false));
            let shown_headers = shown(&reused.prepared.headers);
            assert_eq!(
                shown_headers.get("Authorization").map(String::as_str),
                Some("Bearer resolved-token")
            );
        }
        Route::Stream(_) => panic!("a sync call is not a stream"),
    }
}

fn assert_forced_scope(params: &Object) {
    assert_eq!(
        params.get("azure_scope").and_then(Json::as_str),
        Some("https://ai.azure.com/.default")
    );
    assert_eq!(
        params.get("tenant_id").and_then(Json::as_str),
        Some("tenant-a")
    );
    assert_eq!(
        params.get("client_id").and_then(Json::as_str),
        Some("client-a")
    );
    assert_eq!(
        params.get("client_secret").and_then(Json::as_str),
        Some("secret-a")
    );
    assert_eq!(params.get("extra").and_then(Json::as_str), Some("kept"));
}

fn empty_key_reuses_the_token() {
    let resolver = RecordingResolver::new(Some("resolved-token"));
    let mut fixture = CallFixture::new(BASE);
    fixture.litellm_params = object(r#"{"tenant_id": "tenant-a"}"#);
    fixture.api_key = Some(SecretString::from(String::new()));
    let route = dispatch(fixture.call(), &resolver).expect("a route");
    assert!(resolver.params().is_empty());
    match route {
        Route::Completion(completion_call) => {
            let shown_headers = shown(&completion_call.prepared.headers);
            assert!(!shown_headers.contains_key("Authorization"));
        }
        Route::Stream(_) => panic!("a sync call is not a stream"),
    }
}

fn stream_payload_carries_scope_and_ssl() {
    let resolver = RecordingResolver::new(Some("resolved-token"));
    let mut fixture = CallFixture::new(BASE);
    fixture.litellm_params = object(r#"{"ssl_verify": false}"#);
    fixture.api_key = Some(SecretString::from("explicit-key"));
    fixture.acompletion = true;
    fixture.stream = true;
    let route = dispatch(fixture.call(), &resolver).expect("a route");
    assert!(resolver.params().is_empty());
    let Route::Stream(stream) = route else {
        panic!("a streaming async call is a stream");
    };
    assert_eq!(stream.url, url("/threads/runs"));
    assert_eq!(stream.payload.assistant_id, "asst_1");
    assert!(stream.payload.stream);
    assert_eq!(
        stream
            .payload
            .thread
            .as_ref()
            .map(|thread| thread.messages.len()),
        Some(0)
    );
    assert_eq!(stream.payload.instructions, None);
    assert_eq!(stream.ssl_verify, Json::Bool(false));
    assert_eq!(stream.headers.len(), 2);
}

fn agent_token_missing_value_error() {
    let resolver = RecordingResolver::new(None);
    let fixture = CallFixture::new(BASE);
    let missing_error = dispatch(fixture.call(), &resolver)
        .err()
        .expect("a missing token");
    assert_eq!(missing_error.status(), None);
    assert_eq!(missing_error.to_string(), MISSING_TOKEN);
    assert_eq!(resolver.params().len(), 1);

    let failing_error = dispatch(fixture.call(), &FailingResolver)
        .err()
        .expect("a credential failure");
    assert_eq!(failing_error.status(), None);
    assert_eq!(
        failing_error.to_string(),
        "litellm_params.tenant_id must be a string"
    );
}

fn dispatch_and_client_defaults() {
    route_matrix();
    client_defaults();
    supplied_client_is_reused();
}

fn route_matrix() {
    let resolver = RecordingResolver::new(Some("resolved-token"));
    for (acompletion, stream) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut fixture = CallFixture::new(BASE);
        fixture.litellm_params = object(r#"{"ssl_verify": true}"#);
        fixture.acompletion = acompletion;
        fixture.stream = stream;
        let route = dispatch(fixture.call(), &resolver).expect("a route");
        assert_route(route, &fixture);
    }
}

fn assert_route(route: Route<'_>, fixture: &CallFixture) {
    if fixture.acompletion && fixture.stream {
        assert!(matches!(route, Route::Stream(_)));
        return;
    }
    let expected = if fixture.acompletion {
        AgentRunMode::Async
    } else {
        AgentRunMode::Sync
    };
    let Route::Completion(picked) = route else {
        panic!("only acompletion with stream picks the stream");
    };
    assert_eq!(picked.mode, expected);
    assert_eq!(picked.ssl_verify, Json::Bool(true));
}

fn client_defaults() {
    let resolver = RecordingResolver::new(Some("resolved-token"));
    let mut sync_fixture = CallFixture::new(BASE);
    sync_fixture.litellm_params = object(r#"{"ssl_verify": false}"#);
    assert_client_kind(
        &resolver,
        &sync_fixture,
        FactoryCall {
            kind: "sync",
            ssl_verify: Json::Bool(false),
        },
    );
    let mut async_fixture = CallFixture::new(BASE);
    async_fixture.acompletion = true;
    assert_client_kind(
        &resolver,
        &async_fixture,
        FactoryCall {
            kind: "async",
            ssl_verify: Json::Null,
        },
    );
}

fn assert_client_kind(resolver: &RecordingResolver, fixture: &CallFixture, expected: FactoryCall) {
    let Route::Completion(completion_call) = dispatch(fixture.call(), resolver).expect("a route")
    else {
        panic!("a non-streaming call is not a stream");
    };
    let outcome = finish(&completion_call, completed_steps());
    assert_eq!(outcome.factory_calls, vec![expected]);
    assert_eq!(reply_of(&outcome).choices[0].message.content, "hello");
}

fn supplied_client_is_reused() {
    let supplied = prepared(&Object::new(), None);
    let scripted = transport(completed_steps());
    let given: &dyn AgentRunTransport = &scripted;
    let sleeper = RecordingSleeper::default();
    let optional_params = Object::new();
    let call = CompletionCall {
        model: MODEL,
        messages: &[],
        optional_params: &optional_params,
        prepared: supplied,
        ssl_verify: Json::Null,
        reply: AgentReply::default(),
        mode: AgentRunMode::Async,
        supplied: Some(given),
    };
    let result = block_on(completion(&call, &UnusedFactory, &sleeper, &FixedCounter));
    assert_eq!(result.expect("a reply").choices[0].message.content, "hello");
    assert!(sleeper.waits().is_empty());
    assert_eq!(scripted.state().requests().len(), 4);
}

#[rstest]
#[case::new_thread_run_flow(new_thread_run_flow)]
#[case::existing_thread_message_flow(existing_thread_message_flow)]
#[case::failed_cancelled_expired_run(failed_cancelled_expired_run)]
#[case::nonexpected_status(nonexpected_status)]
#[case::sixty_polls_408(sixty_polls_408)]
#[case::agent_entra_forced_scope(agent_entra_forced_scope)]
#[case::agent_token_missing_value_error(agent_token_missing_value_error)]
#[case::dispatch_and_client_defaults(dispatch_and_client_defaults)]
fn agent_runs_cases(#[case] check: fn()) {
    check();
}
