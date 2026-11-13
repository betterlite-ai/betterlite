use std::io::Write as _;
use std::iter::repeat_with;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use betterlite_models::decisions::Json;
use flate2::Compression;
use flate2::write::GzEncoder;
use rstest::rstest;
use tokio_util::task::TaskTracker;

use super::support::{
    Answer, ClientSetup, FakeClock, FakeLangfuse, PUBLIC_KEY, SECRET_KEY, api_error, chat_prompt,
    client_for, client_from, header, prompt_version, proxied, text_prompt,
};
use crate::langfuse_rest::errors::LangfuseRestError;
use crate::langfuse_rest::records::{Prompt, PromptMessage};
use crate::langfuse_rest::transport::RequestFailure;

const TTL: Duration = Duration::from_secs(60);
const NOT_FOUND: &str = r#"{"message":"prompt not found"}"#;

#[tokio::test]
async fn a_prompt_is_fetched_with_its_version_and_label() {
    let body = text_prompt("greeting", 3);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let prompt = client
        .get_prompt("greeting", Some("production"), Some(3))
        .await
        .expect("the prompt");
    assert_eq!(prompt_version(&prompt), 3);
    assert_eq!(
        server.requests()[0].lines().next().expect("a request line"),
        "GET /api/public/v2/prompts/greeting?version=3&label=production HTTP/1.1"
    );
}

#[tokio::test]
async fn a_prompt_without_a_label_or_version_sends_no_query() {
    let body = text_prompt("greeting", 1);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    assert_eq!(
        server.requests()[0].lines().next().expect("a request line"),
        "GET /api/public/v2/prompts/greeting HTTP/1.1"
    );
}

#[tokio::test]
async fn the_prompt_name_is_percent_encoded_in_the_path() {
    let name = "folder/greeting \u{fc}";
    let body = text_prompt(name, 1);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _prompt = client
        .get_prompt(name, None, None)
        .await
        .expect("the prompt");
    assert_eq!(
        server.requests()[0].lines().next().expect("a request line"),
        "GET /api/public/v2/prompts/folder%2Fgreeting%20%C3%BC HTTP/1.1"
    );
}

#[tokio::test]
async fn unreserved_prompt_name_characters_stay_literal() {
    let name = "azAZ09-._~";
    let body = text_prompt(name, 1);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _prompt = client
        .get_prompt(name, None, None)
        .await
        .expect("the prompt");
    assert_eq!(
        server.requests()[0].lines().next().expect("a request line"),
        "GET /api/public/v2/prompts/azAZ09-._~ HTTP/1.1"
    );
}

#[tokio::test]
async fn a_server_error_is_retried_once() {
    let body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 500,
            body: r#"{"message":"boom"}"#.to_owned(),
        },
        Answer::Respond {
            status: 200,
            body: body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    assert_eq!(prompt_version(&prompt), 2);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_transport_error_is_retried_once() {
    let body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(vec![
        Answer::Close,
        Answer::Respond {
            status: 200,
            body: body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    assert_eq!(prompt_version(&prompt), 2);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_body_that_ends_early_is_retried_once() {
    let body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(vec![
        Answer::Truncated {
            body: b"short".to_vec(),
            headers: Vec::new(),
        },
        Answer::Respond {
            status: 200,
            body: body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    assert_eq!(prompt_version(&prompt), 2);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_gzipped_body_that_ends_early_is_retried_once() {
    let body = text_prompt("greeting", 2);
    let compressed = gzipped(&body);
    let prefix = compressed.get(..10).expect("the gzip header");
    let server = FakeLangfuse::start(vec![
        Answer::Truncated {
            body: prefix.to_vec(),
            headers: vec![("content-encoding", "gzip".to_owned())],
        },
        Answer::Respond {
            status: 200,
            body: body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    assert_eq!(prompt_version(&prompt), 2);
    assert_eq!(server.request_count(), 2);
}

fn gzipped(text: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes()).expect("the gzip body");
    encoder.finish().expect("the gzip body")
}

#[tokio::test]
async fn two_server_errors_raise_the_last_status() {
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 500,
            body: r#"{"message":"first"}"#.to_owned(),
        },
        Answer::Respond {
            status: 503,
            body: r#"{"message":"second"}"#.to_owned(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = api_error(
        client
            .get_prompt("greeting", None, None)
            .await
            .expect_err("the prompt fails"),
    );
    assert_eq!(error.status_code, 503);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn two_transport_errors_keep_the_transport_failure() {
    let server = FakeLangfuse::start(vec![Answer::Close, Answer::Close]).await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the prompt fails");
    assert!(
        matches!(
            &error,
            LangfuseRestError::Request(RequestFailure::Transport(_))
        ),
        "{error:?}"
    );
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_client_error_is_not_retried() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 400,
        body: r#"{"message":"bad"}"#.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = api_error(
        client
            .get_prompt("greeting", None, None)
            .await
            .expect_err("the prompt fails"),
    );
    assert_eq!(error.status_code, 400);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn a_rate_limit_is_not_retried() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 429,
        body: r#"{"message":"slow down"}"#.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = api_error(
        client
            .get_prompt("greeting", None, None)
            .await
            .expect_err("the prompt fails"),
    );
    assert_eq!(error.status_code, 429);
    assert_eq!(server.request_count(), 1);
}

#[rstest]
#[case::client_error_499(499, 1, false)]
#[case::server_error_500(500, 2, true)]
#[tokio::test]
async fn the_retry_boundary_is_a_server_error(
    #[case] status: u16,
    #[case] expected_requests: usize,
    #[case] succeeds: bool,
) {
    let body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status,
            body: r#"{"message":"nope"}"#.to_owned(),
        },
        Answer::Respond {
            status: 200,
            body: body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let outcome = client.get_prompt("greeting", None, None).await;
    assert_eq!(outcome.is_ok(), succeeds);
    assert_eq!(server.request_count(), expected_requests);
}

#[tokio::test]
async fn a_server_error_is_retried_without_delay() {
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 500,
            body: r#"{"message":"first"}"#.to_owned(),
        },
        Answer::Respond {
            status: 500,
            body: r#"{"message":"second"}"#.to_owned(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let started = Instant::now();
    let _error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the prompt fails");
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "the retry slept instead of running at once"
    );
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn an_api_error_carries_the_status_and_body() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 404,
        body: NOT_FOUND.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = api_error(
        client
            .get_prompt("greeting", None, None)
            .await
            .expect_err("the prompt fails"),
    );
    assert_eq!(error.status_code, 404);
    let expected: Json = serde_json::from_str(NOT_FOUND).expect("the fixture");
    assert_eq!(error.body, expected);
}

#[tokio::test]
async fn an_api_error_never_carries_response_headers() {
    let server = FakeLangfuse::start(vec![Answer::RespondWith {
        status: 404,
        body: NOT_FOUND.to_owned(),
        headers: vec![("x-langfuse-secret", "leaky-value".to_owned())],
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the prompt fails");
    let rendered = error.to_string();
    assert!(!rendered.contains("leaky-value"), "{rendered}");
    assert!(!rendered.contains("x-langfuse-secret"), "{rendered}");
    let debugged = format!("{error:?}");
    assert!(!debugged.contains("leaky-value"), "{debugged}");
}

#[tokio::test]
async fn a_success_status_with_an_unreadable_body_is_an_api_error() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: "not json".to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = api_error(
        client
            .get_prompt("greeting", None, None)
            .await
            .expect_err("the prompt fails"),
    );
    assert_eq!(error.status_code, 200);
    assert_eq!(error.body, Json::Text("not json".to_owned()));
}

#[tokio::test]
async fn a_body_that_is_not_utf8_is_not_retried() {
    let server = FakeLangfuse::start(vec![Answer::Raw {
        status: 500,
        body: vec![0xff, 0xfe],
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the prompt fails");
    let LangfuseRestError::Body { status, .. } = &error else {
        panic!("expected a body failure, got {error:?}")
    };
    assert_eq!(*status, 500);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn a_body_that_fails_content_decoding_is_not_retried() {
    let server = FakeLangfuse::start(vec![Answer::RespondWith {
        status: 200,
        body: String::from("not gzip at all"),
        headers: vec![("content-encoding", "gzip".to_owned())],
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the prompt fails");
    assert!(
        matches!(
            &error,
            LangfuseRestError::Request(RequestFailure::Decode(_))
        ),
        "{error:?}"
    );
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn a_response_that_is_not_the_declared_prompt_shape_is_a_decode_error() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: r#"{"type":"text"}"#.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the prompt fails");
    assert!(matches!(error, LangfuseRestError::Decode { .. }));
}

#[tokio::test]
async fn a_text_prompt_keeps_its_template_and_metadata() {
    let body = text_prompt("greeting", 7);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    let Prompt::Text(text) = prompt else {
        panic!("expected a text prompt")
    };
    assert_eq!(text.prompt, "Hello {{name}}");
    assert_eq!(text.meta.name, "greeting");
    assert_eq!(text.meta.version, 7);
    let expected: Json = serde_json::from_str(r#"{"model":"gpt-4o"}"#).expect("the fixture");
    assert_eq!(text.meta.config, expected);
    assert_eq!(text.meta.labels, vec!["production".to_owned()]);
    assert_eq!(text.meta.tags, vec!["greeting".to_owned()]);
    assert_eq!(text.meta.commit_message.as_deref(), Some("first"));
    assert!(text.meta.resolution_graph.is_some());
}

#[tokio::test]
async fn a_chat_prompt_keeps_its_messages_and_metadata() {
    let body = chat_prompt("greeting", 4);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    let Prompt::Chat(chat) = prompt else {
        panic!("expected a chat prompt")
    };
    assert_eq!(chat.meta.name, "greeting");
    assert_eq!(chat.meta.version, 4);
    assert_eq!(chat.meta.commit_message, None);
    assert_eq!(chat.prompt.len(), 2);
    match &chat.prompt[0] {
        PromptMessage::Chat(message) => {
            assert_eq!(message.role, "system");
            assert_eq!(message.content, "You help");
            assert_eq!(message.kind, None);
        }
        PromptMessage::Placeholder(message) => {
            panic!("expected a chat message, got {message:?}")
        }
    }
    match &chat.prompt[1] {
        PromptMessage::Placeholder(message) => assert_eq!(message.name, "history"),
        PromptMessage::Chat(message) => panic!("expected a placeholder, got {message:?}"),
    }
}

#[tokio::test]
async fn a_redirected_prompt_request_follows_to_the_target() {
    let body = text_prompt("greeting", 1);
    let server = FakeLangfuse::start(vec![
        Answer::RespondWith {
            status: 302,
            body: String::new(),
            headers: vec![("location", "/api/public/v2/prompts/greeting".to_owned())],
        },
        Answer::Respond {
            status: 200,
            body: body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    assert_eq!(prompt_version(&prompt), 1);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_redirect_loop_is_not_retried() {
    let redirect = || Answer::RespondWith {
        status: 302,
        body: String::new(),
        headers: vec![("location", "/api/public/v2/prompts/greeting".to_owned())],
    };
    let script = repeat_with(redirect).take(22).collect();
    let server = FakeLangfuse::start(script).await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the redirect loop fails");
    assert!(
        matches!(
            &error,
            LangfuseRestError::Request(RequestFailure::Transport(_))
        ),
        "{error:?}"
    );
    assert_eq!(server.request_count(), 11);
}

#[tokio::test]
async fn the_configured_timeout_bounds_each_prompt_request() {
    let server = FakeLangfuse::start(vec![
        Answer::Stall(Duration::from_secs(10)),
        Answer::Stall(Duration::from_secs(10)),
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_from(ClientSetup {
        base_url: server.base_url.clone(),
        public_key: Some(PUBLIC_KEY),
        secret_key: Some(SECRET_KEY),
        ttl: TTL,
        timeout: Duration::from_millis(250),
        http: Some(proxied()),
        clock: &clock,
        tasks: &tasks,
    });
    let started = Instant::now();
    let error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the prompt times out");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the configured timeout did not bound the request"
    );
    assert!(matches!(error, LangfuseRestError::Request(_)), "{error:?}");
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn the_rest_client_sends_basic_authorization_and_identity_headers() {
    let body = text_prompt("greeting", 1);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    let head = server.requests().into_iter().next().expect("a request");
    // base64 of "pk-lf-test:sk-lf-test"
    assert_eq!(
        header(&head, "authorization").as_deref(),
        Some("Basic cGstbGYtdGVzdDpzay1sZi10ZXN0")
    );
    assert_eq!(
        header(&head, "x-langfuse-public-key").as_deref(),
        Some(PUBLIC_KEY)
    );
    assert_eq!(
        header(&head, "x-langfuse-sdk-name").as_deref(),
        Some("python")
    );
    assert_eq!(
        header(&head, "x-langfuse-sdk-version").as_deref(),
        Some("4.15.2")
    );
    assert_eq!(header(&head, "x-fern-language").as_deref(), Some("Python"));
}

#[tokio::test]
async fn missing_keys_send_no_authorization_header() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 401,
        body: NOT_FOUND.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_from(ClientSetup {
        base_url: server.base_url.clone(),
        public_key: None,
        secret_key: None,
        ttl: TTL,
        timeout: Duration::from_secs(5),
        http: Some(proxied()),
        clock: &clock,
        tasks: &tasks,
    });
    assert!(client.auth_check().await.is_some());
    let head = server.requests().into_iter().next().expect("a request");
    assert_eq!(header(&head, "authorization"), None);
    assert_eq!(header(&head, "x-langfuse-public-key"), None);
    assert_eq!(
        header(&head, "x-langfuse-sdk-name").as_deref(),
        Some("python")
    );
}

#[tokio::test]
async fn a_public_key_without_a_secret_sends_no_authorization_header() {
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 401,
        body: NOT_FOUND.to_owned(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_from(ClientSetup {
        base_url: server.base_url.clone(),
        public_key: Some(PUBLIC_KEY),
        secret_key: None,
        ttl: TTL,
        timeout: Duration::from_secs(5),
        http: Some(proxied()),
        clock: &clock,
        tasks: &tasks,
    });
    assert!(client.auth_check().await.is_some());
    let head = server.requests().into_iter().next().expect("a request");
    assert_eq!(header(&head, "authorization"), None);
    assert_eq!(
        header(&head, "x-langfuse-public-key").as_deref(),
        Some(PUBLIC_KEY)
    );
}

#[tokio::test]
async fn a_public_key_a_header_cannot_carry_is_reported_at_request_time() {
    let server = FakeLangfuse::start(Vec::new()).await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_from(ClientSetup {
        base_url: server.base_url.clone(),
        public_key: Some("pk\nlf"),
        secret_key: Some(SECRET_KEY),
        ttl: TTL,
        timeout: Duration::from_secs(5),
        http: Some(proxied()),
        clock: &clock,
        tasks: &tasks,
    });
    let failure = client.auth_check().await.expect("the check fails");
    assert!(
        failure.reason.contains("x-langfuse-public-key"),
        "{}",
        failure.reason
    );
    let error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the prompt fails");
    assert!(
        error.to_string().contains("x-langfuse-public-key"),
        "{error}"
    );
    assert_eq!(server.request_count(), 0);
}

#[tokio::test]
async fn a_client_without_an_injected_http_client_reaches_the_server() {
    let body = text_prompt("greeting", 1);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_from(ClientSetup {
        base_url: server.base_url.clone(),
        public_key: Some(PUBLIC_KEY),
        secret_key: Some(SECRET_KEY),
        ttl: TTL,
        timeout: Duration::from_secs(5),
        http: None,
        clock: &clock,
        tasks: &tasks,
    });
    let prompt = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the prompt");
    assert_eq!(prompt_version(&prompt), 1);
}

#[tokio::test]
async fn a_client_reports_an_invalid_base_url_as_a_transport_failure() {
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_from(ClientSetup {
        base_url: "not a url".to_owned(),
        public_key: Some(PUBLIC_KEY),
        secret_key: Some(SECRET_KEY),
        ttl: TTL,
        timeout: Duration::from_secs(5),
        http: Some(proxied()),
        clock: &clock,
        tasks: &tasks,
    });
    let error = client
        .get_prompt("greeting", None, None)
        .await
        .expect_err("the prompt fails");
    assert!(
        matches!(
            &error,
            LangfuseRestError::Request(RequestFailure::Url { .. })
        ),
        "{error:?}"
    );
}
