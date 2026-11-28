#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::media::error::MediaError;
use betterlite_prompts::media_fetch::error::ImageFetchError;
use rstest::rstest;
use secrecy::SecretString;
use tempfile::NamedTempFile;
use tracing::Level;
use tracing::subscriber::with_default;

use crate::cache_lookup::target::CacheProvider;
use crate::gemini_call::GeminiReply;
use crate::gemini_call::error::GeminiCallError;
use crate::gemini_http::error::{GeminiHttpError, GeminiStreamError};
use crate::gemini_request::error::RequestError;
use crate::gemini_stream::error::StreamError;
use crate::model_policy::capabilities::VertexCapabilityProvider;
use crate::vertex_embedding_call::post::{ClientTimeout, HANDLER_DEFAULT_TIMEOUT, Timeout};
use betterlite_core::api::vector_store_http_write::service::ClientKind;

use super::support::{
    ANSWER, BODY, REMOTE_TOOL_MEDIA, Reply, SCHEMA_ERROR, STREAM_CHUNK, Setup, Stage, TOKEN,
    Variation, auth_failure_fixture, complete_reply, error_reply, line, pooled_fixture,
    stream_reply, supplied_fixture,
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
    assert_eq!(
        serde_json::from_slice::<Json>(&posts[0].body).expect("the posted body is JSON"),
        serde_json::from_str::<Json>(BODY).expect("the expected body is JSON")
    );
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
    assert_eq!(
        serde_json::from_slice::<Json>(&opens[0].body).expect("the opened body is JSON"),
        serde_json::from_str::<Json>(BODY).expect("the expected body is JSON")
    );
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
    assert_eq!(
        serde_json::from_slice::<Json>(&posts[0].body).expect("the posted body is JSON"),
        serde_json::from_str::<Json>(BODY).expect("the expected body is JSON")
    );
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
    ClientKind::Asynchronous,
    ClientTimeout::Explicit(Timeout {
        total: THIRTY_SECONDS,
        connect: THIRTY_SECONDS
    })
)]
#[case::supplied_sync_client_keeps_its_timeout(ClientKind::Sync, ClientTimeout::HandlerDefault)]
fn ordered_effects_supplied_client_matches_the_sync_branch(
    #[case] kind: ClientKind,
    #[case] expected: ClientTimeout,
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
#[case::supplied_async_client_opens_on_the_pooled_transport(ClientKind::Asynchronous)]
fn ordered_effects_supplied_client_matches_the_stream_branch(#[case] kind: ClientKind) {
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
    #[case] timeout: Option<Duration>,
    #[case] expected: ClientTimeout,
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
    #[case] timeout: Option<Duration>,
    #[case] expected: Duration,
) {
    let fixture = pooled_fixture(Vec::new(), Vec::new());
    let setup = Setup {
        acompletion: false,
        stream: None,
        timeout,
    };
    assert_eq!(fixture.cache_deadline(setup), expected);
}

#[rstest]
#[case::vertex(VertexCapabilityProvider::VertexAi, CacheProvider::Vertex)]
#[case::vertex_beta(VertexCapabilityProvider::VertexAiBeta, CacheProvider::VertexBeta)]
#[case::gemini(VertexCapabilityProvider::Gemini, CacheProvider::Gemini)]
fn ordered_effects_sync_cache_provider_follows_the_provider(
    #[case] provider: VertexCapabilityProvider,
    #[case] expected: CacheProvider,
) {
    let fixture = pooled_fixture(Vec::new(), Vec::new());
    assert_eq!(fixture.cache_provider(provider), expected);
}

#[rstest]
#[case::vertex_ai(VertexCapabilityProvider::VertexAi)]
#[case::vertex_ai_beta(VertexCapabilityProvider::VertexAiBeta)]
fn ordered_effects_sync_vertex_providers_share_the_target(
    #[case] provider: VertexCapabilityProvider,
) {
    let fixture = pooled_fixture(vec![complete_reply(200, ANSWER)], Vec::new());
    let variation = Variation {
        provider: Some(provider),
        location: None,
        api_base: None,
        gemini_api_key: None,
        extra_headers: BTreeMap::new(),
        use_psc_endpoint_format: None,
    };
    let reply = fixture.run_with(
        Setup {
            acompletion: false,
            stream: None,
            timeout: None,
        },
        &variation,
    );
    assert!(matches!(reply, Ok(GeminiReply::Complete(_))));
    assert_eq!(fixture.recorder.posts()[0].url, VERTEX_URL);
}

#[test]
fn ordered_effects_sync_completion_reports_a_bad_body_as_a_provider_error() {
    let fixture = pooled_fixture(vec![complete_reply(200, "not json")], Vec::new());
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: None,
        timeout: None,
    });
    assert!(matches!(
        reply,
        Err(GeminiCallError::Http(GeminiHttpError::Provider(_)))
    ));
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

#[rstest]
#[case::async_completion(true)]
#[case::sync_completion(false)]
fn ordered_effects_missing_credentials_stop_before_the_transform(#[case] acompletion: bool) {
    let fixture = auth_failure_fixture();
    let reply = fixture.run(Setup {
        acompletion,
        stream: None,
        timeout: None,
    });
    assert!(matches!(
        reply,
        Err(GeminiCallError::Http(GeminiHttpError::Credentials { .. }))
    ));
    assert_eq!(fixture.order.stages(), vec![Stage::Auth]);
    assert_eq!(fixture.loads(), 1);
    assert!(fixture.recorder.posts().is_empty());
    assert!(fixture.recorder.opens().is_empty());
}

#[test]
fn ordered_effects_sync_completion_accepts_a_status_inside_the_two_hundreds() {
    let fixture = pooled_fixture(vec![complete_reply(299, ANSWER)], Vec::new());
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: None,
        timeout: None,
    });
    assert!(matches!(reply, Ok(GeminiReply::Complete(_))));
}

#[rstest]
#[case::first_status_outside(300)]
#[case::redirect(302)]
fn ordered_effects_sync_completion_rejects_a_status_outside_the_two_hundreds(#[case] status: u16) {
    let fixture = pooled_fixture(vec![complete_reply(status, ANSWER)], Vec::new());
    let reply = fixture.run(Setup {
        acompletion: false,
        stream: None,
        timeout: None,
    });
    let Err(GeminiCallError::Http(GeminiHttpError::Provider(provider))) = reply else {
        panic!("a non-2xx reply is the provider's error");
    };
    assert_eq!(provider.status_code, status);
}

#[test]
fn ordered_effects_sync_stream_refuses_a_status_with_the_provider_error() {
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
    assert!(provider.message.ends_with(SCHEMA_ERROR));
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

#[rstest]
#[case::async_complete(true, None)]
#[case::async_stream(true, Some(true))]
#[case::sync_complete(false, None)]
#[case::sync_stream(false, Some(true))]
fn ordered_effects_target_error_precedes_the_transform(
    #[case] acompletion: bool,
    #[case] stream: Option<bool>,
) {
    let fixture = pooled_fixture(Vec::new(), Vec::new());
    let variation = Variation {
        provider: None,
        location: Some("us central1"),
        api_base: None,
        gemini_api_key: None,
        extra_headers: BTreeMap::new(),
        use_psc_endpoint_format: None,
    };
    let reply = fixture.run_with(
        Setup {
            acompletion,
            stream,
            timeout: None,
        },
        &variation,
    );
    assert!(matches!(
        reply,
        Err(GeminiCallError::Http(GeminiHttpError::Target { .. }))
    ));
    assert_eq!(fixture.order.stages(), vec![Stage::Auth]);
    assert_eq!(fixture.cache_calls(), 0);
    assert!(fixture.recorder.posts().is_empty());
    assert!(fixture.recorder.opens().is_empty());
}

// LiteLLM's async branch inlines remote media before the body (L2717, L2810); its sync branch downloads it during conversion
#[rstest]
#[case::async_fetches_before_the_send(true, 1, 0)]
#[case::sync_fetches_before_the_send(false, 1, 0)]
fn ordered_effects_remote_tool_media_follows_the_branch_transform(
    #[case] acompletion: bool,
    #[case] fetches: usize,
    #[case] posts: usize,
) {
    let fixture = pooled_fixture(vec![complete_reply(200, ANSWER)], Vec::new())
        .with_messages(REMOTE_TOOL_MEDIA);
    let reply = fixture.run(Setup {
        acompletion,
        stream: None,
        timeout: None,
    });
    let Err(GeminiCallError::Request(RequestError::Inline(MediaError::Fetch(
        ImageFetchError::Rejected { .. },
    )))) = reply
    else {
        panic!("a failed download stops the call");
    };
    assert_eq!(fixture.fetches(), fetches);
    assert_eq!(fixture.recorder.posts().len(), posts);
    assert!(fixture.recorder.opens().is_empty());
}

#[test]
fn ordered_effects_sync_completion_posts_the_caller_headers() {
    let fixture = pooled_fixture(vec![complete_reply(200, ANSWER)], Vec::new());
    let variation = Variation {
        provider: None,
        location: None,
        api_base: None,
        gemini_api_key: None,
        extra_headers: BTreeMap::from([("x-goog-user-project".to_owned(), "other".to_owned())]),
        use_psc_endpoint_format: None,
    };
    let reply = fixture.run_with(
        Setup {
            acompletion: false,
            stream: None,
            timeout: None,
        },
        &variation,
    );
    assert!(matches!(reply, Ok(GeminiReply::Complete(_))));
    let posts = fixture.recorder.posts();
    assert_eq!(
        posts[0]
            .headers
            .get("x-goog-user-project")
            .map(String::as_str),
        Some("other")
    );
    assert_eq!(
        posts[0].headers.get("Authorization"),
        Some(&format!("Bearer {TOKEN}"))
    );
}

#[rstest]
#[case::psc_flag_set(
    Some(true),
    "https://psc.example.com/v1/projects/project-p/locations/us-central1/endpoints/gemini-2.5-flash:generateContent"
)]
#[case::psc_flag_absent(
    None,
    "https://psc.example.com/v1/projects/project-p/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
)]
fn ordered_effects_sync_completion_reads_the_psc_flag(
    #[case] flag: Option<bool>,
    #[case] expected: &str,
) {
    let fixture = pooled_fixture(vec![complete_reply(200, ANSWER)], Vec::new());
    let variation = Variation {
        provider: None,
        location: None,
        api_base: Some("https://psc.example.com"),
        gemini_api_key: None,
        extra_headers: BTreeMap::new(),
        use_psc_endpoint_format: flag,
    };
    let reply = fixture.run_with(
        Setup {
            acompletion: false,
            stream: None,
            timeout: None,
        },
        &variation,
    );
    assert!(matches!(reply, Ok(GeminiReply::Complete(_))));
    assert_eq!(fixture.recorder.posts()[0].url, expected);
}

#[test]
fn ordered_effects_sync_gemini_posts_to_studio_with_the_api_key() {
    let fixture = pooled_fixture(vec![complete_reply(200, ANSWER)], Vec::new());
    let variation = Variation {
        provider: Some(VertexCapabilityProvider::Gemini),
        location: None,
        api_base: None,
        gemini_api_key: Some(SecretString::from("studio-key")),
        extra_headers: BTreeMap::new(),
        use_psc_endpoint_format: None,
    };
    let reply = fixture.run_with(
        Setup {
            acompletion: false,
            stream: None,
            timeout: None,
        },
        &variation,
    );
    assert!(matches!(reply, Ok(GeminiReply::Complete(_))));
    let posts = fixture.recorder.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(
        posts[0].url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(
        posts[0].headers.get("x-goog-api-key").map(String::as_str),
        Some("studio-key")
    );
    assert!(!posts[0].headers.contains_key("Authorization"));
    assert_eq!(fixture.loads(), 0);
    assert!(!fixture.order.stages().contains(&Stage::Auth));
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
