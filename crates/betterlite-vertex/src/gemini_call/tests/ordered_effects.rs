#![cfg(test)]

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;
use tempfile::NamedTempFile;
use tracing::Level;
use tracing::subscriber::with_default;

use crate::gemini_call::GeminiReply;
use crate::gemini_call::error::GeminiCallError;
use crate::gemini_http::call::ClientKind;
use crate::gemini_http::error::{GeminiHttpError, GeminiStreamError};
use crate::gemini_stream::error::StreamError;
use crate::vertex_embedding_call::post::{ClientTimeout, HANDLER_DEFAULT_TIMEOUT, Timeout};

use super::support::{
    ANSWER, BODY, Reply, SCHEMA_ERROR, STREAM_CHUNK, Setup, Stage, TOKEN, complete_reply,
    error_reply, line, pooled_fixture, stream_reply, supplied_fixture,
};

const VERTEX_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/project-p/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent";
const STREAM_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1/projects/project-p/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse";
const THIRTY_SECONDS: Duration = Duration::from_secs(30);

#[test]
fn ordered_effects_sync_completion_authenticates_transforms_then_posts() {
    let fixture = pooled_fixture(vec![complete_reply(200, ANSWER)], Vec::new());
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: None,
        timeout: None,
    });
    let Ok(GeminiReply::Complete(response)) = reply else {
        panic!("a synchronous call answers with a complete response");
    };
    assert_eq!(response.model, "gemini-2.5-flash");
    assert_eq!(response.usage.total_tokens, 5);
    assert_eq!(
        fixture.order.stages(),
        vec![Stage::Auth, Stage::Transform, Stage::Send]
    );
    assert_eq!(fixture.loads(), 1);
    assert_eq!(fixture.cache_calls(), 0);
    let posts = fixture.recorder.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, VERTEX_URL);
    assert_eq!(posts[0].body.as_slice(), BODY.as_bytes());
    assert_eq!(
        posts[0].headers.get("Authorization"),
        Some(&format!("Bearer {TOKEN}"))
    );
    assert_eq!(posts[0].timeout, ClientTimeout::HandlerDefault);
    assert!(fixture.recorder.opens().is_empty());
}

#[test]
fn ordered_effects_sync_stream_opens_only_when_iterated() {
    let fixture = pooled_fixture(
        Vec::new(),
        vec![stream_reply(200, vec![line(STREAM_CHUNK)])],
    );
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: Some(true),
        timeout: Some(THIRTY_SECONDS),
    });
    let Ok(GeminiReply::Stream(mut stream)) = reply else {
        panic!("a streamed synchronous call answers with a generated stream");
    };
    assert_eq!(fixture.order.stages(), vec![Stage::Auth, Stage::Transform]);
    assert!(fixture.recorder.posts().is_empty());
    assert!(fixture.recorder.opens().is_empty());
    let chunk = fixture
        .runtime
        .block_on(stream.next_chunk())
        .expect("a first chunk")
        .expect("a decoded chunk");
    assert_eq!(chunk.id, "r1");
    assert_eq!(
        fixture.order.stages(),
        vec![Stage::Auth, Stage::Transform, Stage::Open]
    );
    let opens = fixture.recorder.opens();
    assert_eq!(opens.len(), 1);
    assert_eq!(opens[0].url, STREAM_URL);
    assert_eq!(opens[0].body.as_slice(), BODY.as_bytes());
    assert_eq!(
        opens[0].headers.get("Authorization"),
        Some(&format!("Bearer {TOKEN}"))
    );
    // LiteLLM's make_sync_call partial carries no timeout, so the caller's value never reaches the stream
    assert_eq!(opens[0].timeout, ClientTimeout::HandlerDefault);
    assert!(fixture.recorder.posts().is_empty());
    let ended = fixture.runtime.block_on(stream.next_chunk());
    assert!(ended.is_none(), "the stream ends after its last chunk");
}

#[test]
fn ordered_effects_async_completion_authenticates_once() {
    let fixture = pooled_fixture(vec![complete_reply(200, ANSWER)], Vec::new());
    let reply = fixture.run(Setup {
        acompletion: true,
        stream: None,
        timeout: None,
    });
    let Ok(GeminiReply::Complete(response)) = reply else {
        panic!("an asynchronous call answers with a complete response");
    };
    assert_eq!(response.usage.total_tokens, 5);
    assert_eq!(
        fixture.order.stages(),
        vec![Stage::Auth, Stage::Transform, Stage::Send]
    );
    assert_eq!(fixture.loads(), 1);
    let posts = fixture.recorder.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].url, VERTEX_URL);
    assert_eq!(posts[0].body.as_slice(), BODY.as_bytes());
    assert_eq!(posts[0].timeout, ClientTimeout::HandlerDefault);
    assert!(fixture.recorder.opens().is_empty());
}

#[test]
fn ordered_effects_async_stream_defers_the_open() {
    let fixture = pooled_fixture(
        Vec::new(),
        vec![stream_reply(200, vec![line(STREAM_CHUNK)])],
    );
    let reply = fixture.run(Setup {
        acompletion: true,
        stream: Some(true),
        timeout: None,
    });
    let Ok(GeminiReply::Stream(mut stream)) = reply else {
        panic!("a streamed asynchronous call answers with a generated stream");
    };
    assert_eq!(fixture.order.stages(), vec![Stage::Auth, Stage::Transform]);
    assert!(fixture.recorder.opens().is_empty());
    let chunk = fixture
        .runtime
        .block_on(stream.next_chunk())
        .expect("a first chunk")
        .expect("a decoded chunk");
    assert_eq!(chunk.id, "r1");
    assert_eq!(
        fixture.order.stages(),
        vec![Stage::Auth, Stage::Transform, Stage::Open]
    );
    assert_eq!(fixture.recorder.opens()[0].url, STREAM_URL);
    assert!(fixture.recorder.posts().is_empty());
}

#[rstest]
#[case::supplied_async_client_is_replaced(
    ClientKind::Async,
    ClientTimeout::Explicit(Timeout {
        total: THIRTY_SECONDS,
        connect: THIRTY_SECONDS
    })
)]
#[case::supplied_sync_client_keeps_its_timeout(ClientKind::Sync, ClientTimeout::HandlerDefault)]
fn ordered_effects_supplied_client_matches_the_sync_branch(
    kind: ClientKind,
    expected: ClientTimeout,
) {
    let fixture = supplied_fixture(
        kind,
        (vec![complete_reply(200, ANSWER)], Vec::new()),
        (vec![complete_reply(200, ANSWER)], Vec::new()),
    );
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: None,
        timeout: Some(THIRTY_SECONDS),
    });
    let Ok(GeminiReply::Complete(_)) = reply else {
        panic!("a synchronous call answers with a complete response");
    };
    let caller = fixture.caller.as_ref().expect("a supplied client").posts();
    let pooled = fixture.recorder.posts();
    let (used, unused) = if kind == ClientKind::Sync {
        (caller, pooled)
    } else {
        (pooled, caller)
    };
    assert_eq!(used.len(), 1);
    assert_eq!(used[0].timeout, expected);
    assert!(unused.is_empty());
}

#[rstest]
#[case::supplied_sync_client_opens_on_its_own_transport(ClientKind::Sync)]
#[case::supplied_async_client_opens_on_the_pooled_transport(ClientKind::Async)]
fn ordered_effects_supplied_client_matches_the_stream_branch(kind: ClientKind) {
    let fixture = supplied_fixture(
        kind,
        (
            Vec::new(),
            vec![stream_reply(200, vec![line(STREAM_CHUNK)])],
        ),
        (
            Vec::new(),
            vec![stream_reply(200, vec![line(STREAM_CHUNK)])],
        ),
    );
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: Some(true),
        timeout: None,
    });
    let Ok(GeminiReply::Stream(mut stream)) = reply else {
        panic!("a streamed synchronous call answers with a generated stream");
    };
    let caller = fixture.caller.as_ref().expect("a supplied client");
    assert!(caller.opens().is_empty() && fixture.recorder.opens().is_empty());
    let chunk = fixture
        .runtime
        .block_on(stream.next_chunk())
        .expect("a first chunk")
        .expect("a decoded chunk");
    assert_eq!(chunk.id, "r1");
    let caller_opens = caller.opens();
    let pooled_opens = fixture.recorder.opens();
    let (used, unused) = if kind == ClientKind::Sync {
        (caller_opens, pooled_opens)
    } else {
        (pooled_opens, caller_opens)
    };
    assert_eq!(used.len(), 1);
    assert_eq!(used[0].url, STREAM_URL);
    assert!(unused.is_empty());
}

#[rstest]
#[case::explicit_zero_timeout(
    Some(Duration::ZERO),
    ClientTimeout::Explicit(Timeout {
        total: Duration::ZERO,
        connect: Duration::ZERO
    })
)]
#[case::explicit_timeout(
    Some(THIRTY_SECONDS),
    ClientTimeout::Explicit(Timeout {
        total: THIRTY_SECONDS,
        connect: THIRTY_SECONDS
    })
)]
#[case::absent_timeout(None, ClientTimeout::HandlerDefault)]
fn ordered_effects_sync_completion_carries_the_timeout(
    timeout: Option<Duration>,
    expected: ClientTimeout,
) {
    let fixture = pooled_fixture(vec![complete_reply(200, ANSWER)], Vec::new());
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: None,
        timeout,
    });
    assert!(matches!(reply, Ok(GeminiReply::Complete(_))));
    assert_eq!(fixture.recorder.posts()[0].timeout, expected);
}

#[rstest]
#[case::explicit_timeout(Some(THIRTY_SECONDS), THIRTY_SECONDS)]
#[case::absent_timeout(None, HANDLER_DEFAULT_TIMEOUT.total)]
fn ordered_effects_sync_cache_deadline_follows_the_call_timeout(
    timeout: Option<Duration>,
    expected: Duration,
) {
    let fixture = pooled_fixture(Vec::new(), Vec::new());
    let setup = Setup {
        acompletion: false,
        stream: None,
        timeout,
    };
    assert_eq!(fixture.cache_deadline(setup), expected);
}

#[test]
fn ordered_effects_sync_completion_keeps_the_provider_status() {
    let fixture = pooled_fixture(
        vec![error_reply(
            400,
            &[("x-goog-request-id", "abc")],
            SCHEMA_ERROR,
        )],
        Vec::new(),
    );
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: None,
        timeout: None,
    });
    let Err(GeminiCallError::Http(GeminiHttpError::Provider(provider))) = reply else {
        panic!("a 400 is the provider's own error");
    };
    assert_eq!(provider.status_code, 400);
    assert_eq!(provider.message, SCHEMA_ERROR);
    assert_eq!(
        provider
            .headers
            .as_ref()
            .and_then(|headers| headers.get("x-goog-request-id"))
            .map(String::as_str),
        Some("abc")
    );
    assert_eq!(
        fixture.order.stages(),
        vec![Stage::Auth, Stage::Transform, Stage::Send]
    );
}

#[test]
fn ordered_effects_sync_completion_turns_a_timeout_into_408() {
    let fixture = pooled_fixture(vec![Reply::Timeout], Vec::new());
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: None,
        timeout: None,
    });
    let Err(GeminiCallError::Http(GeminiHttpError::Timeout(provider))) = reply else {
        panic!("a client timeout is a 408 provider error");
    };
    assert_eq!(provider.status_code, 408);
    assert_eq!(provider.message, "Timeout error occurred.");
    assert!(provider.headers.is_none());
}

#[test]
fn ordered_effects_sync_stream_refuses_a_status_without_translating() {
    let fixture = pooled_fixture(
        Vec::new(),
        vec![stream_reply(400, vec![SCHEMA_ERROR.as_bytes().to_vec()])],
    );
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: Some(true),
        timeout: None,
    });
    let Ok(GeminiReply::Stream(mut stream)) = reply else {
        panic!("a streamed synchronous call answers with a generated stream");
    };
    let chunk = fixture.runtime.block_on(stream.next_chunk());
    assert!(chunk.is_some(), "the refusal reaches the first chunk");
    let Some(Err(GeminiStreamError::Http(GeminiHttpError::Provider(provider)))) = chunk else {
        panic!("a 400 is the provider's own error");
    };
    assert_eq!(provider.status_code, 400);
    assert_eq!(provider.message, SCHEMA_ERROR);
}

#[test]
fn ordered_effects_sync_stream_surfaces_a_chunk_error_once() {
    let fixture = pooled_fixture(Vec::new(), vec![stream_reply(200, vec![line("[1]")])]);
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: Some(true),
        timeout: None,
    });
    let Ok(GeminiReply::Stream(mut stream)) = reply else {
        panic!("a streamed synchronous call answers with a generated stream");
    };
    let chunk = fixture.runtime.block_on(stream.next_chunk());
    assert!(matches!(
        chunk,
        Some(Err(GeminiStreamError::Chunk(StreamError::NotAnObject)))
    ));
    let after = fixture.runtime.block_on(stream.next_chunk());
    assert!(after.is_none(), "a failed stream stays finished");
}

#[test]
fn ordered_effects_sync_completion_logs_the_request() {
    let fixture = pooled_fixture(vec![complete_reply(200, ANSWER)], Vec::new());
    let log = NamedTempFile::new().expect("a log file");
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .with_writer(Arc::new(log.reopen().expect("a log writer")))
        .finish();
    let reply = with_default(subscriber, || {
        fixture.run(Setup {
            acompletion: false,
            stream: None,
            timeout: None,
        })
    });
    assert!(matches!(reply, Ok(GeminiReply::Complete(_))));
    let logged = fs::read_to_string(log.path()).expect("the captured log");
    assert!(logged.contains("sending the Gemini request"));
    assert!(logged.contains("gemini-2.5-flash"));
    assert!(logged.contains("us-central1-aiplatform.googleapis.com"));
    assert!(!logged.contains(TOKEN));
}
