#![cfg(test)]

pub(crate) mod fakes;

mod a2a_stream_tests_behavior {
    use std::collections::BTreeMap;
    use std::mem;
    use std::sync::Mutex;
    use std::time::Duration;

    use betterlite_a2a::send_params::build_message_send_params;
    use betterlite_core::agents::json::AgentJson;
    use futures_util::StreamExt as _;
    use futures_util::stream::{iter, once};
    use rstest::rstest;
    use serde_json::json;
    use tokio::time::sleep;

    use super::fakes::{CLOSED, CountingConverter, Reply, client, serve_agent};
    use crate::agent_endpoints::a2a_stream::error::{StreamAbort, StreamError, StreamFailure};
    use crate::agent_endpoints::a2a_stream::event_source::EventSourceRequest;
    use crate::agent_endpoints::a2a_stream::ports::{
        HookedStream, JsonStream, StreamHooks, StreamOpen, UpstreamAgentStream,
    };
    use crate::agent_endpoints::a2a_stream::{
        FrameStream, MessageStreamRequest, SseResponse, relay_message_stream,
        relay_task_subscription, sse_response,
    };

    const KEEPALIVE: [(&str, &str); 2] =
        [("Cache-Control", "no-cache"), ("X-Accel-Buffering", "no")];
    const NO_HEADERS: [(&str, &str); 0] = [];

    struct ScriptedUpstream {
        items: Mutex<Vec<Result<AgentJson, StreamError>>>,
        opened: Mutex<Vec<StreamOpen>>,
    }

    impl UpstreamAgentStream for ScriptedUpstream {
        fn stream(&self, open: StreamOpen) -> JsonStream {
            self.opened.lock().unwrap().push(open);
            iter(mem::take(&mut *self.items.lock().unwrap())).boxed()
        }
    }

    // after a failure it sends one more event, so a test sees whether the relay stopped
    struct ScriptedHooks {
        failure: Option<StreamFailure>,
    }

    impl StreamHooks for ScriptedHooks {
        fn serve(&self, events: JsonStream) -> HookedStream {
            let tail = match &self.failure {
                Some(failure) => vec![Err(failure.clone()), Ok(json!({"after": "failure"}))],
                None => Vec::new(),
            };
            events
                .map(|item| {
                    item.map_err(|error| StreamFailure::Failed {
                        message: format!("hooked: {error}"),
                    })
                })
                .chain(iter(tail))
                .boxed()
        }
    }

    fn upstream(items: Vec<Result<AgentJson, StreamError>>) -> ScriptedUpstream {
        ScriptedUpstream {
            items: Mutex::new(items),
            opened: Mutex::new(Vec::new()),
        }
    }

    fn message_params() -> AgentJson {
        json!({"message": {"messageId": "m1", "role": "user", "parts": [{"kind": "text", "text": "hi"}]}})
    }

    fn message_request(params: AgentJson) -> MessageStreamRequest {
        MessageStreamRequest {
            request_id: json!("r1"),
            params,
            api_base: Some(String::from("http://agent.test")),
            litellm_params: Some(BTreeMap::from([(String::from("model"), json!("a2a/x"))])),
            agent_id: Some(String::from("agent-1")),
            metadata: Some(json!({"user": "u1"})),
            proxy_server_request: Some(json!({"url": "/a2a/agent-1"})),
            agent_extra_headers: Some(vec![(String::from("x-agent"), String::from("1"))]),
        }
    }

    fn event_of(frame: &str) -> AgentJson {
        let payload = frame.strip_prefix("data: ").unwrap();
        serde_json::from_str(payload.strip_suffix("\n\n").unwrap()).unwrap()
    }

    async fn observed(response: SseResponse) -> Vec<Result<AgentJson, String>> {
        let frames: Vec<Result<String, StreamAbort>> = response.body.take(8).collect().await;
        frames
            .into_iter()
            .map(|frame| match frame {
                Ok(text) => Ok(event_of(&text)),
                Err(abort) => Err(abort.message),
            })
            .collect()
    }

    fn expected(outputs: &[Result<&str, &str>]) -> Vec<Result<AgentJson, String>> {
        outputs
            .iter()
            .map(|output| match output {
                Ok(text) => Ok(serde_json::from_str(text).unwrap()),
                Err(message) => Err((*message).to_owned()),
            })
            .collect()
    }

    fn late_frame() -> FrameStream {
        once(async {
            sleep(Duration::from_millis(1500)).await;
            Ok(String::from("data: 1\n\n"))
        })
        .boxed()
    }

    #[rstest]
    #[case::one_second(Some(1.0_f64), &KEEPALIVE, &[": ping\n\n", "data: 1\n\n"])]
    #[case::longer_than_the_gap(Some(2.0_f64), &KEEPALIVE, &["data: 1\n\n"])]
    #[case::unset(None, &NO_HEADERS, &["data: 1\n\n"])]
    #[case::zero(Some(0.0_f64), &NO_HEADERS, &["data: 1\n\n"])]
    #[case::negative(Some(-1.0_f64), &NO_HEADERS, &["data: 1\n\n"])]
    #[case::not_a_number(Some(f64::NAN), &NO_HEADERS, &["data: 1\n\n"])]
    #[case::infinite(Some(f64::INFINITY), &NO_HEADERS, &["data: 1\n\n"])]
    #[tokio::test(start_paused = true)]
    async fn sse_response_pings_only_for_a_finite_positive_interval(
        #[case] interval: Option<f64>,
        #[case] headers: &[(&str, &str)],
        #[case] frames: &[&str],
    ) {
        let response = sse_response(late_frame(), interval);
        assert_eq!(response.headers, headers);
        let body: Vec<String> = response.body.take(8).map(Result::unwrap).collect().await;
        assert_eq!(body, frames);
    }

    #[rstest]
    #[case::not_an_object(json!("text"))]
    #[case::missing_parts(json!({"message": {"messageId": "m1", "role": "user"}}))]
    #[tokio::test]
    async fn invalid_params_answer_one_frame_without_pings(#[case] params: AgentJson) {
        let message = format!(
            "Invalid params: {}",
            build_message_send_params(&params).unwrap_err()
        );
        let agent = upstream(vec![Ok(json!({"a": 1_i32}))]);
        let response = relay_message_stream(
            &agent,
            &CountingConverter,
            None::<&ScriptedHooks>,
            message_request(params),
            Some(1.0_f64),
        );
        assert!(response.headers.is_empty());
        let event = json!({"jsonrpc": "2.0", "id": "r1", "error": {"code": -32_602_i64, "message": message}});
        assert_eq!(observed(response).await, vec![Ok(event)]);
        assert!(agent.opened.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn message_stream_opens_the_upstream_with_the_parsed_params() {
        let agent = upstream(Vec::new());
        let response = relay_message_stream(
            &agent,
            &CountingConverter,
            None::<&ScriptedHooks>,
            message_request(message_params()),
            Some(1.0_f64),
        );
        assert_eq!(response.headers, KEEPALIVE);
        assert_eq!(observed(response).await, Vec::new());
        let opened = agent.opened.lock().unwrap().pop().unwrap();
        assert_eq!(opened.request_id, json!("r1"));
        assert_eq!(
            opened.params,
            build_message_send_params(&message_params()).unwrap()
        );
        assert_eq!(opened.api_base.as_deref(), Some("http://agent.test"));
        assert_eq!(
            opened.litellm_params,
            Some(BTreeMap::from([(String::from("model"), json!("a2a/x"))]))
        );
        assert_eq!(opened.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(opened.metadata, Some(json!({"user": "u1"})));
        assert_eq!(
            opened.proxy_server_request,
            Some(json!({"url": "/a2a/agent-1"}))
        );
        assert_eq!(
            opened.agent_extra_headers,
            Some(vec![(String::from("x-agent"), String::from("1"))])
        );
    }

    #[tokio::test]
    async fn message_stream_frames_each_chunk_after_the_converter() {
        let agent = upstream(vec![
            Ok(json!({"a": 1_i32})),
            Ok(json!("{\"b\": 2}")),
            Ok(json!("plain")),
            Ok(json!([1_i32])),
        ]);
        let response = relay_message_stream(
            &agent,
            &CountingConverter,
            None::<&ScriptedHooks>,
            message_request(message_params()),
            None,
        );
        assert!(response.headers.is_empty());
        let frames: Vec<String> = response.body.map(Result::unwrap).collect().await;
        assert_eq!(
            frames.get(2).map(String::as_str),
            Some("data: \"plain\"\n\n")
        );
        let events: Vec<AgentJson> = frames.iter().map(String::as_str).map(event_of).collect();
        assert_eq!(
            events,
            vec![
                json!({"a": 1_i32, "conversions": 1_i32, "request_id": "r1"}),
                json!({"b": 2_i32, "conversions": 1_i32, "request_id": "r1"}),
                json!("plain"),
                json!([1_i32]),
            ]
        );
    }

    const CONVERTED: &str = r#"{"a": 1, "conversions": 1, "request_id": "r1"}"#;
    const STREAMING_ERROR: &str = r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": "Streaming error: the A2A agent sent a line longer than 1 bytes"}}"#;
    const HOOKED_ERROR: &str = r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": "hooked: the A2A agent sent a line longer than 1 bytes"}}"#;
    const BLOCKED: &str =
        r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": "blocked"}}"#;

    fn failed(message: &str) -> Option<ScriptedHooks> {
        Some(ScriptedHooks {
            failure: Some(StreamFailure::Failed {
                message: message.to_owned(),
            }),
        })
    }

    fn rejected() -> Option<ScriptedHooks> {
        Some(ScriptedHooks {
            failure: Some(StreamFailure::Rejected {
                status: 403,
                detail: String::from("denied"),
            }),
        })
    }

    #[rstest]
    #[case::error_without_hooks(true, None, &[Ok(CONVERTED), Ok(STREAMING_ERROR)])]
    #[case::error_through_hooks(true, Some(ScriptedHooks { failure: None }), &[Ok(CONVERTED), Ok(HOOKED_ERROR)])]
    #[case::hooks_pass_events(false, Some(ScriptedHooks { failure: None }), &[Ok(CONVERTED)])]
    #[case::hook_failure(false, failed("blocked"), &[Ok(CONVERTED), Ok(BLOCKED)])]
    #[case::hook_rejection(false, rejected(), &[Ok(CONVERTED), Err("denied")])]
    #[tokio::test]
    async fn message_stream_ends_on_the_first_error(
        #[case] upstream_fails: bool,
        #[case] hooks: Option<ScriptedHooks>,
        #[case] outputs: &[Result<&str, &str>],
    ) {
        let mut items = vec![Ok(json!({"a": 1_i32}))];
        if upstream_fails {
            items.push(Err(StreamError::LineTooLong { limit: 1 }));
            items.push(Ok(json!({"late": 1_i32})));
        }
        let agent = upstream(items);
        let request = message_request(message_params());
        let response = match hooks {
            Some(scripted) => {
                relay_message_stream(&agent, &CountingConverter, Some(&scripted), request, None)
            }
            None => relay_message_stream(
                &agent,
                &CountingConverter,
                None::<&ScriptedHooks>,
                request,
                None,
            ),
        };
        assert_eq!(observed(response).await, expected(outputs));
    }

    const SOURCE_EVENT: &str = r#"{"a": 1, "conversions": 1, "request_id": "r1"}"#;
    const UNNAMED_EVENT: &str = r#"{"a": 1, "conversions": 1, "request_id": null}"#;
    const UNREACHABLE: &str = "the A2A agent could not be reached";
    const HOOKED_UNREACHABLE: &str = r#"{"jsonrpc": "2.0", "id": "r1", "error": {"code": -32603, "message": "hooked: the A2A agent could not be reached"}}"#;
    const BLOCKED_UNNAMED: &str =
        r#"{"jsonrpc": "2.0", "id": null, "error": {"code": -32603, "message": "blocked"}}"#;

    #[rstest]
    #[case::events_without_hooks(false, Some("r1"), None, &[Ok(SOURCE_EVENT), Ok("[1, 2]")])]
    #[case::source_error_without_hooks(true, Some("r1"), None, &[Err(UNREACHABLE)])]
    #[case::events_through_hooks(false, Some("r1"), Some(ScriptedHooks { failure: None }), &[Ok(SOURCE_EVENT), Ok("[1, 2]")])]
    #[case::source_error_through_hooks(true, Some("r1"), Some(ScriptedHooks { failure: None }), &[Ok(HOOKED_UNREACHABLE)])]
    #[case::hook_failure_without_request_id(false, None, failed("blocked"), &[Ok(UNNAMED_EVENT), Ok("[1, 2]"), Ok(BLOCKED_UNNAMED)])]
    #[case::hook_rejection(false, Some("r1"), rejected(), &[Ok(SOURCE_EVENT), Ok("[1, 2]"), Err("denied")])]
    #[tokio::test]
    async fn task_subscription_frames_the_source_events(
        #[case] unreachable: bool,
        #[case] request_id: Option<&str>,
        #[case] hooks: Option<ScriptedHooks>,
        #[case] outputs: &[Result<&str, &str>],
    ) {
        let agent = serve_agent(Reply {
            status: 200,
            headers: "",
            parts: vec![b"data: {\"a\": 1}\n\ndata: [1, 2]\n\n"],
            hold_open: false,
        })
        .await;
        let request = EventSourceRequest {
            agent_url: if unreachable {
                CLOSED.to_owned()
            } else {
                agent.url.clone()
            },
            body: json!({"jsonrpc": "2.0", "id": request_id, "method": "tasks/resubscribe"}),
            request_id: request_id.map(AgentJson::from),
            extra_headers: Vec::new(),
        };
        let ping = Some(30.0_f64);
        let response = match hooks {
            Some(scripted) => relay_task_subscription(
                &client(),
                &CountingConverter,
                Some(&scripted),
                request,
                ping,
            ),
            None => relay_task_subscription(
                &client(),
                &CountingConverter,
                None::<&ScriptedHooks>,
                request,
                ping,
            ),
        };
        assert_eq!(response.headers, KEEPALIVE);
        assert_eq!(observed(response).await, expected(outputs));
    }

    #[tokio::test]
    async fn task_subscription_frames_an_event_as_its_json_text() {
        let agent = serve_agent(Reply {
            status: 200,
            headers: "",
            parts: vec![b"data: [1, 2]\n\ndata: \"s\"\n\n"],
            hold_open: false,
        })
        .await;
        let request = EventSourceRequest {
            agent_url: agent.url.clone(),
            body: json!({}),
            request_id: None,
            extra_headers: Vec::new(),
        };
        let response = relay_task_subscription(
            &client(),
            &CountingConverter,
            None::<&ScriptedHooks>,
            request,
            None,
        );
        assert!(response.headers.is_empty());
        let frames: Vec<String> = response.body.map(Result::unwrap).collect().await;
        assert_eq!(frames, ["data: [1,2]\n\n", "data: \"s\"\n\n"]);
    }
}
