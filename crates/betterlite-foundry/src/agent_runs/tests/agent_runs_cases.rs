#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;

use crate::agent_requests::prepare::Headers;
use crate::agent_results::reply::{AgentReply, Usage};
use crate::agent_runs::decode::{RUN_CANCELLED, RUN_EXPIRED, RUN_FAILED};
use crate::agent_runs::dispatch::{DispatchCall, Route, dispatch};
use crate::agent_runs::error::AgentRunError;
use crate::agent_runs::flow::{CompletionCall, MAX_POLL_ATTEMPTS, POLL_INTERVAL, completion};
use crate::agent_runs::transport::{AgentRunMethod, AgentRunMode, AgentRunTransport};

use super::support::{
    BASE, FactoryCall, FailingResolver, FixedCounter, MODEL, Outcome, RecordedRequest,
    RecordingResolver, RecordingSleeper, Run, SECRET, Step, UnusedFactory, block_on, execute,
    failed_step, finish, messages, object, prepared, repeated, shown, status_step, step, transport,
};

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

fn new_thread_run_flow() {
    let mut case = Run::sync(vec![
        step(201, r#"{"id": "thread-1", "object": "thread"}"#),
        step(200, r#"{"id": "msg-1"}"#),
        step(201, r#"{"id": "run-1", "status": "queued"}"#),
        status_step("completed"),
        hello_messages(),
    ]);
    case.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let sync_outcome = execute(&case);
    assert!(sync_outcome.factory_calls.is_empty());
    assert!(sync_outcome.sleeps.is_empty());
    let sync_state = sync_outcome.transport.state();
    let requests = sync_state.requests();
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
    assert_eq!(sync_state.remaining(), 0);
    drop(sync_state);
    let reply = reply_of(&sync_outcome);
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
    let async_outcome = execute(&created);
    assert_eq!(
        async_outcome.factory_calls,
        vec![FactoryCall {
            kind: "async",
            ssl_verify: Json::Null,
        }]
    );
    let async_state = async_outcome.transport.state();
    assert_eq!(async_state.requests().len(), 5);
    assert_eq!(async_state.requests()[0].url, url("/threads"));
    assert_eq!(
        async_state.requests()[3].url,
        url("/threads/thread-2/runs/run-2")
    );
    assert_eq!(async_state.remaining(), 0);
    drop(async_state);
    assert_eq!(reply_of(&async_outcome).choices[0].message.content, "async");

    for thread_id in [r#"{"thread_id": null}"#, r#"{"thread_id": ""}"#] {
        let mut falsey = Run::sync(vec![
            step(201, r#"{"id": "thread-3"}"#),
            step(201, r#"{"id": "run-3"}"#),
            status_step("completed"),
            hello_messages(),
        ]);
        falsey.optional_params = object(thread_id);
        let falsey_outcome = execute(&falsey);
        let falsey_state = falsey_outcome.transport.state();
        let falsey_requests = falsey_state.requests();
        assert_eq!(falsey_requests.len(), 4);
        assert_request(
            &falsey_requests[0],
            AgentRunMethod::Post,
            &url("/threads"),
            None,
        );
        assert_request(
            &falsey_requests[1],
            AgentRunMethod::Post,
            &url("/threads/thread-3/runs"),
            Some(&object(r#"{"assistant_id": "asst_1"}"#)),
        );
        assert_eq!(
            reply_of(&falsey_outcome).choices[0].message.content,
            "hello"
        );
    }
}

fn existing_thread_message_flow() {
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
    let outcome = execute(&case);
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
    assert_request(&requests[7], AgentRunMethod::Get, &message_url, None);
    assert_eq!(state.remaining(), 0);
    drop(state);
    assert_eq!(reply_of(&outcome).choices[0].message.content, "done");

    let variants = [
        (
            r#"{"thread_id": "thread-9", "instructions": null}"#,
            r#"{"assistant_id": "asst_1", "instructions": null}"#,
        ),
        (
            r#"{"thread_id": "thread-9", "instructions": []}"#,
            r#"{"assistant_id": "asst_1", "instructions": []}"#,
        ),
    ];
    for (optional_params, run_body) in variants {
        let mut variant = Run::sync(vec![
            step(200, r#"{"id": "run-1"}"#),
            status_step("completed"),
            hello_messages(),
        ]);
        variant.optional_params = object(optional_params);
        let variant_outcome = execute(&variant);
        let variant_state = variant_outcome.transport.state();
        assert_request(
            &variant_state.requests()[0],
            AgentRunMethod::Post,
            &url("/threads/thread-9/runs"),
            Some(&object(run_body)),
        );
    }

    let mut absent = Run::sync(vec![
        step(200, r#"{"id": "run-1"}"#),
        status_step("completed"),
        hello_messages(),
    ]);
    absent.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let absent_outcome = execute(&absent);
    let absent_state = absent_outcome.transport.state();
    assert_request(
        &absent_state.requests()[0],
        AgentRunMethod::Post,
        &url("/threads/thread-9/runs"),
        Some(&object(r#"{"assistant_id": "asst_1"}"#)),
    );
}

fn failed_cancelled_expired_run() {
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
        let terminal_outcome = execute(&case);
        let error = failure_of(&terminal_outcome);
        assert_eq!(error.status(), Some(500));
        assert_eq!(error.to_string(), expected);
        assert!(terminal_outcome.sleeps.is_empty());
        let terminal_state = terminal_outcome.transport.state();
        assert_eq!(terminal_state.requests().len(), 2);
        assert_eq!(terminal_state.remaining(), 0);
    }

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
        let unknown_outcome = execute(&case);
        let unknown_error = failure_of(&unknown_outcome);
        assert_eq!(unknown_error.status(), Some(500));
        assert_eq!(unknown_error.to_string(), "Run failed: Unknown error");
        assert!(unknown_outcome.sleeps.is_empty());
    }

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
    let polling_outcome = execute(&polling);
    assert_eq!(polling_outcome.sleeps, vec![POLL_INTERVAL; 4]);
    let polling_state = polling_outcome.transport.state();
    assert_eq!(polling_state.requests().len(), 7);
    assert_eq!(polling_state.remaining(), 0);
    drop(polling_state);
    assert_eq!(
        reply_of(&polling_outcome).choices[0].message.content,
        "hello"
    );
}

fn nonexpected_status() {
    let mut create = Run::sync(vec![step(500, "thread broke")]);
    create.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let create_outcome = execute(&create);
    let create_error = failure_of(&create_outcome);
    assert_eq!(create_error.status(), Some(500));
    assert_eq!(
        create_error.to_string(),
        "Failed to create thread: thread broke"
    );
    assert_eq!(create_outcome.transport.state().requests().len(), 1);
    assert!(create_outcome.sleeps.is_empty());

    let mut message = Run::sync(vec![step(400, "bad message")]);
    message.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    message.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let message_outcome = execute(&message);
    let message_error = failure_of(&message_outcome);
    assert_eq!(message_error.status(), Some(400));
    assert_eq!(
        message_error.to_string(),
        "Failed to add message: bad message"
    );
    assert_eq!(message_outcome.transport.state().requests().len(), 1);

    let mut run = Run::sync(vec![step(429, "slow down")]);
    run.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let run_outcome = execute(&run);
    let run_error = failure_of(&run_outcome);
    assert_eq!(run_error.status(), Some(429));
    assert_eq!(run_error.to_string(), "Failed to create run: slow down");
    assert_eq!(run_outcome.transport.state().requests().len(), 1);

    let mut status = Run::sync(vec![
        step(201, r#"{"id": "run-1"}"#),
        step(503, "status down"),
    ]);
    status.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let status_outcome = execute(&status);
    let status_error = failure_of(&status_outcome);
    assert_eq!(status_error.status(), Some(503));
    assert_eq!(
        status_error.to_string(),
        "Failed to get run status: status down"
    );
    assert_eq!(status_outcome.transport.state().requests().len(), 2);
    assert!(status_outcome.sleeps.is_empty());

    let mut list = Run::sync(vec![
        step(201, r#"{"id": "run-1"}"#),
        status_step("completed"),
        step(500, "messages broke"),
    ]);
    list.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let list_outcome = execute(&list);
    let list_error = failure_of(&list_outcome);
    assert_eq!(list_error.status(), Some(500));
    assert_eq!(
        list_error.to_string(),
        "Failed to get messages: messages broke"
    );
    assert_eq!(list_outcome.transport.state().requests().len(), 3);

    let mut binary = Run::sync(vec![Step {
        status: 500,
        body: vec![0xff, 0xfe],
    }]);
    binary.messages = messages(r#"[{"role": "user", "content": "hi"}]"#);
    let binary_outcome = execute(&binary);
    let binary_error = failure_of(&binary_outcome);
    assert_eq!(binary_error.status(), Some(500));
    assert_eq!(
        binary_error.to_string(),
        "Failed to create thread: the response body is not UTF-8"
    );
}

fn sixty_polls_408() {
    let mut steps = vec![step(201, r#"{"id": "run-1"}"#)];
    steps.extend(repeated(MAX_POLL_ATTEMPTS, &status_step("in_progress")));
    let mut timed_out = Run::sync(steps);
    timed_out.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let timed_out_outcome = execute(&timed_out);
    let timeout_error = failure_of(&timed_out_outcome);
    assert_eq!(timeout_error.status(), Some(408));
    assert_eq!(
        timeout_error.to_string(),
        "Run timed out waiting for completion"
    );
    assert_eq!(timed_out_outcome.sleeps.len(), MAX_POLL_ATTEMPTS);
    assert!(
        timed_out_outcome
            .sleeps
            .iter()
            .all(|wait| *wait == POLL_INTERVAL)
    );
    let timed_out_state = timed_out_outcome.transport.state();
    assert_eq!(
        timed_out_state.requests().len(),
        MAX_POLL_ATTEMPTS.saturating_add(1)
    );
    assert_eq!(timed_out_state.remaining(), 0);
    drop(timed_out_state);

    let mut immediate = Run::sync(vec![
        step(201, r#"{"id": "run-1"}"#),
        status_step("completed"),
        hello_messages(),
    ]);
    immediate.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let immediate_outcome = execute(&immediate);
    assert!(immediate_outcome.sleeps.is_empty());
    let immediate_state = immediate_outcome.transport.state();
    assert_eq!(immediate_state.requests().len(), 3);
    assert_eq!(
        immediate_state.requests()[1].url,
        url("/threads/thread-9/runs/run-1")
    );
    assert_eq!(
        immediate_state.requests()[2].url,
        url("/threads/thread-9/messages")
    );
    drop(immediate_state);

    let mut steps = vec![step(201, r#"{"id": "run-1"}"#)];
    steps.extend(repeated(
        MAX_POLL_ATTEMPTS.saturating_sub(1),
        &status_step("in_progress"),
    ));
    steps.push(status_step("completed"));
    steps.push(hello_messages());
    let mut last = Run::sync(steps);
    last.optional_params = object(r#"{"thread_id": "thread-9"}"#);
    let last_outcome = execute(&last);
    assert_eq!(
        last_outcome.sleeps.len(),
        MAX_POLL_ATTEMPTS.saturating_sub(1)
    );
    assert_eq!(reply_of(&last_outcome).choices[0].message.content, "hello");
}

fn agent_entra_forced_scope() {
    let resolver = RecordingResolver::new(Some("resolved-token"));
    let optional_params = Object::new();
    let litellm_params = object(
        r#"{"tenant_id": "tenant-a", "client_id": "client-a", "client_secret": "secret-a",
            "azure_scope": "https://wrong.example/.default", "ssl_verify": false, "extra": "kept"}"#,
    );
    let headers = Headers::new();
    let environment = BTreeMap::new();
    let call = DispatchCall {
        model: MODEL,
        messages: &[],
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        headers: &headers,
        environment: &environment,
        api_base: Some(BASE),
        api_key: None,
        reply: AgentReply::default(),
        acompletion: false,
        stream: false,
    };
    let reused_route = dispatch(call, &resolver).expect("a route");
    let params = resolver.params();
    assert_eq!(params.len(), 1);
    assert_eq!(
        params[0].get("azure_scope").and_then(Json::as_str),
        Some("https://ai.azure.com/.default")
    );
    assert_eq!(
        params[0].get("tenant_id").and_then(Json::as_str),
        Some("tenant-a")
    );
    assert_eq!(
        params[0].get("client_id").and_then(Json::as_str),
        Some("client-a")
    );
    assert_eq!(
        params[0].get("client_secret").and_then(Json::as_str),
        Some("secret-a")
    );
    assert_eq!(params[0].get("extra").and_then(Json::as_str), Some("kept"));
    assert_eq!(
        litellm_params.get("azure_scope").and_then(Json::as_str),
        Some("https://wrong.example/.default")
    );
    match reused_route {
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

    let empty = SecretString::from(String::new());
    let empty_call = DispatchCall {
        model: MODEL,
        messages: &[],
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        headers: &headers,
        environment: &environment,
        api_base: Some(BASE),
        api_key: Some(&empty),
        reply: AgentReply::default(),
        acompletion: false,
        stream: false,
    };
    let empty_route = dispatch(empty_call, &resolver).expect("a route");
    assert_eq!(resolver.params().len(), 1);
    match empty_route {
        Route::Completion(empty_completion) => {
            let shown_headers = shown(&empty_completion.prepared.headers);
            assert!(!shown_headers.contains_key("Authorization"));
        }
        Route::Stream(_) => panic!("a sync call is not a stream"),
    }

    let key = SecretString::from("explicit-key");
    let stream_call = DispatchCall {
        model: MODEL,
        messages: &[],
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        headers: &headers,
        environment: &environment,
        api_base: Some(BASE),
        api_key: Some(&key),
        reply: AgentReply::default(),
        acompletion: true,
        stream: true,
    };
    let stream_route = dispatch(stream_call, &resolver).expect("a route");
    assert_eq!(resolver.params().len(), 1);
    match stream_route {
        Route::Stream(stream) => {
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
        Route::Completion(_) => panic!("a streaming async call is a stream"),
    }
}

fn agent_token_missing_value_error() {
    let resolver = RecordingResolver::new(None);
    let optional_params = Object::new();
    let litellm_params = Object::new();
    let headers = Headers::new();
    let environment = BTreeMap::new();
    let missing_call = DispatchCall {
        model: MODEL,
        messages: &[],
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        headers: &headers,
        environment: &environment,
        api_base: Some(BASE),
        api_key: None,
        reply: AgentReply::default(),
        acompletion: false,
        stream: false,
    };
    let missing_error = dispatch(missing_call, &resolver)
        .err()
        .expect("a missing token");
    assert_eq!(missing_error.status(), None);
    assert_eq!(
        missing_error.to_string(),
        "api_key (Azure AD token) is required for Azure Foundry Agents. Either pass api_key \
         directly, or set AZURE_TENANT_ID, AZURE_CLIENT_ID, and AZURE_CLIENT_SECRET environment \
         variables for Service Principal auth. Manual token: az account get-access-token \
         --resource 'https://ai.azure.com'"
    );
    assert_eq!(resolver.params().len(), 1);

    let failing = FailingResolver;
    let failing_call = DispatchCall {
        model: MODEL,
        messages: &[],
        optional_params: &optional_params,
        litellm_params: &litellm_params,
        headers: &headers,
        environment: &environment,
        api_base: Some(BASE),
        api_key: None,
        reply: AgentReply::default(),
        acompletion: false,
        stream: false,
    };
    let failing_error = dispatch(failing_call, &failing)
        .err()
        .expect("a credential failure");
    assert_eq!(failing_error.status(), None);
    assert_eq!(
        failing_error.to_string(),
        "litellm_params.tenant_id must be a string"
    );
}

fn dispatch_and_client_defaults() {
    let resolver = RecordingResolver::new(Some("resolved-token"));
    let optional_params = Object::new();
    let headers = Headers::new();
    let environment = BTreeMap::new();

    let routes = [(false, false), (false, true), (true, false), (true, true)];
    for (acompletion, stream) in routes {
        let litellm_params = object(r#"{"ssl_verify": true}"#);
        let call = DispatchCall {
            model: MODEL,
            messages: &[],
            optional_params: &optional_params,
            litellm_params: &litellm_params,
            headers: &headers,
            environment: &environment,
            api_base: Some(BASE),
            api_key: None,
            reply: AgentReply::default(),
            acompletion,
            stream,
        };
        let route = dispatch(call, &resolver).expect("a route");
        if acompletion && stream {
            assert!(matches!(route, Route::Stream(_)));
        } else {
            match route {
                Route::Completion(picked) => {
                    let expected = if acompletion {
                        AgentRunMode::Async
                    } else {
                        AgentRunMode::Sync
                    };
                    assert_eq!(picked.mode, expected);
                    assert_eq!(picked.ssl_verify, Json::Bool(true));
                }
                Route::Stream(_) => panic!("only acompletion with stream picks the stream"),
            }
        }
    }

    let sync_litellm = object(r#"{"ssl_verify": false}"#);
    let sync_call = DispatchCall {
        model: MODEL,
        messages: &[],
        optional_params: &optional_params,
        litellm_params: &sync_litellm,
        headers: &headers,
        environment: &environment,
        api_base: Some(BASE),
        api_key: None,
        reply: AgentReply::default(),
        acompletion: false,
        stream: false,
    };
    let Route::Completion(sync_completion) = dispatch(sync_call, &resolver).expect("a route")
    else {
        panic!("a sync call is not a stream");
    };
    let sync_outcome = finish(
        &sync_completion,
        vec![
            step(201, r#"{"id": "thread-1"}"#),
            step(201, r#"{"id": "run-1"}"#),
            status_step("completed"),
            hello_messages(),
        ],
    );
    assert_eq!(
        sync_outcome.factory_calls,
        vec![FactoryCall {
            kind: "sync",
            ssl_verify: Json::Bool(false),
        }]
    );
    assert_eq!(reply_of(&sync_outcome).choices[0].message.content, "hello");

    let async_litellm = Object::new();
    let async_call = DispatchCall {
        model: MODEL,
        messages: &[],
        optional_params: &optional_params,
        litellm_params: &async_litellm,
        headers: &headers,
        environment: &environment,
        api_base: Some(BASE),
        api_key: None,
        reply: AgentReply::default(),
        acompletion: true,
        stream: false,
    };
    let Route::Completion(async_completion) = dispatch(async_call, &resolver).expect("a route")
    else {
        panic!("a non-streaming async call is not a stream");
    };
    let async_outcome = finish(
        &async_completion,
        vec![
            step(201, r#"{"id": "thread-1"}"#),
            step(201, r#"{"id": "run-1"}"#),
            status_step("completed"),
            hello_messages(),
        ],
    );
    assert_eq!(
        async_outcome.factory_calls,
        vec![FactoryCall {
            kind: "async",
            ssl_verify: Json::Null,
        }]
    );

    let supplied = prepared(&Object::new(), None);
    let scripted = transport(vec![
        step(201, r#"{"id": "thread-1"}"#),
        step(201, r#"{"id": "run-1"}"#),
        status_step("completed"),
        hello_messages(),
    ]);
    let given: &dyn AgentRunTransport = &scripted;
    let sleeper = RecordingSleeper::default();
    let supplied_call = CompletionCall {
        model: MODEL,
        messages: &[],
        optional_params: &optional_params,
        prepared: supplied,
        ssl_verify: Json::Null,
        reply: AgentReply::default(),
        mode: AgentRunMode::Async,
        supplied: Some(given),
    };
    let supplied_result = block_on(completion(
        &supplied_call,
        &UnusedFactory,
        &sleeper,
        &FixedCounter,
    ));
    assert_eq!(
        supplied_result.expect("a reply").choices[0].message.content,
        "hello"
    );
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
