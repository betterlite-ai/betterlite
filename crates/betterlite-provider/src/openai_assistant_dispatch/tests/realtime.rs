#![cfg(test)]

use std::sync::Arc;

use betterlite_config::tls::Verification;
use betterlite_openai::realtime::error::RealtimeError;
use betterlite_openai::realtime::failure::{ClientAction, RealtimeClose};
use betterlite_openai::realtime::ssl::RealtimeSsl;
use betterlite_openai::realtime::url::QueryParams;
use secrecy::SecretString;

use crate::openai_assistant_dispatch::OpenAiAssistantResult;
use crate::openai_assistant_dispatch::call::{
    OpenAiAssistantOperation, RealtimeCall, RealtimeFailureOwned,
};
use crate::openai_assistant_dispatch::error::OpenAiAssistantError;

use super::support::{BASE, KEY, Recorder, RecordingLogger, run_operation};

fn realtime_params() -> QueryParams {
    QueryParams::from([
        (String::from("model"), Some(String::from("realtime-a"))),
        (String::from("intent"), Some(String::from("transcription"))),
    ])
}

fn realtime_connect(
    api_base: &str,
    beta_realtime: bool,
    ssl_verification: Verification,
) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Realtime(RealtimeCall::Connect {
        api_base: Some(String::from(api_base)),
        api_key: Some(SecretString::from(KEY)),
        model: String::from("realtime-a"),
        query_params: Some(realtime_params()),
        beta_realtime,
        ssl_verification,
        max_message_size_bytes: 4096,
    })
}

async fn realtime_secure_session() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = realtime_connect("https://example.test/", true, Verification::Enabled);
    let prepared = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Session(plan) = prepared else {
        panic!("the session preparation did not return a session")
    };
    assert_eq!(
        plan.url,
        "wss://example.test/v1/realtime?model=realtime-a&intent=transcription"
    );
    assert_eq!(
        plan.headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-fixture")
    );
    assert_eq!(
        plan.headers.get("OpenAI-Beta").map(String::as_str),
        Some("realtime=v1")
    );
    assert_eq!(plan.ssl, RealtimeSsl::Shared);
    assert_eq!(plan.max_message_size_bytes, 4096);
    assert_eq!(
        plan.force_transcription_model.as_deref(),
        Some("realtime-a")
    );
    assert!(!plan.event_normalizer_installed);
    assert_eq!(logger.calls.len(), 1);
    assert_eq!(logger.calls[0].api_base, plan.url);
    assert_eq!(logger.calls[0].query_params, realtime_params());
    assert!(transport.seen.lock().unwrap().is_empty());
}

async fn realtime_plain_session() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = OpenAiAssistantOperation::Realtime(RealtimeCall::Connect {
        api_base: Some(String::from("http://127.0.0.1:8080/base")),
        api_key: Some(SecretString::from(KEY)),
        model: String::from("realtime-a"),
        query_params: None,
        beta_realtime: false,
        ssl_verification: Verification::Enabled,
        max_message_size_bytes: 1024,
    });
    let prepared = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Session(plan) = prepared else {
        panic!("the plain session preparation did not return a session")
    };
    assert_eq!(plan.url, "ws://127.0.0.1:8080/v1/realtime?model=realtime-a");
    assert_eq!(plan.ssl, RealtimeSsl::None);
    assert_eq!(plan.force_transcription_model, None);
    assert_eq!(plan.headers.get("OpenAI-Beta"), None);
    assert_eq!(plan.max_message_size_bytes, 1024);
}

async fn realtime_insecure_session() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = realtime_connect(BASE, false, Verification::Disabled);
    let prepared = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Session(plan) = prepared else {
        panic!("the insecure session preparation did not return a session")
    };
    assert!(plan.url.starts_with("wss://"));
    assert_eq!(plan.ssl, RealtimeSsl::Insecure);
    assert_eq!(plan.headers.get("OpenAI-Beta"), None);
}

async fn realtime_missing_key() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = OpenAiAssistantOperation::Realtime(RealtimeCall::Connect {
        api_base: None,
        api_key: None,
        model: String::from("realtime-a"),
        query_params: None,
        beta_realtime: false,
        ssl_verification: Verification::Enabled,
        max_message_size_bytes: 1024,
    });
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the missing realtime api key did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Realtime(RealtimeError::MissingApiKey)
    ));
}

async fn realtime_default_base() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = OpenAiAssistantOperation::Realtime(RealtimeCall::Connect {
        api_base: None,
        api_key: Some(SecretString::from(KEY)),
        model: String::from("realtime-a"),
        query_params: Some(realtime_params()),
        beta_realtime: false,
        ssl_verification: Verification::Enabled,
        max_message_size_bytes: 4096,
    });
    let prepared = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Session(plan) = prepared else {
        panic!("the default base session did not prepare")
    };
    assert_eq!(
        plan.url,
        "wss://api.openai.com/v1/realtime?model=realtime-a&intent=transcription"
    );
    assert_eq!(plan.ssl, RealtimeSsl::Shared);
}

async fn realtime_connection_failures() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let handshake = OpenAiAssistantOperation::Realtime(RealtimeCall::Failure(
        RealtimeFailureOwned::Handshake { status_code: 403 },
    ));
    let answered = run_operation(&transport, &mut logger, handshake)
        .await
        .unwrap();
    let OpenAiAssistantResult::SessionFailure(refused) = answered else {
        panic!("a handshake refusal did not return a session failure")
    };
    assert_eq!(*refused, ClientAction::RefuseHandshake { status_code: 403 });
    let other =
        OpenAiAssistantOperation::Realtime(RealtimeCall::Failure(RealtimeFailureOwned::Other {
            message: String::from("connection reset"),
        }));
    let failed = run_operation(&transport, &mut logger, other).await.unwrap();
    let OpenAiAssistantResult::SessionFailure(closed) = failed else {
        panic!("a connection failure did not return a session failure")
    };
    assert_eq!(
        *closed,
        ClientAction::Close(RealtimeClose {
            code: 1011,
            reason: String::from("Internal server error: connection reset"),
        })
    );
}

async fn realtime_close_failures() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let ignored = OpenAiAssistantOperation::Realtime(RealtimeCall::Failure(
        RealtimeFailureOwned::CloseFailed {
            message: String::from("connection reset"),
            close_error: String::from("already completed"),
        },
    ));
    let answered = run_operation(&transport, &mut logger, ignored)
        .await
        .unwrap();
    let OpenAiAssistantResult::SessionFailure(action) = answered else {
        panic!("an ignored close failure did not return a session failure")
    };
    assert!(matches!(*action, ClientAction::IgnoreCloseFailure { .. }));
    let unexpected = OpenAiAssistantOperation::Realtime(RealtimeCall::Failure(
        RealtimeFailureOwned::CloseFailed {
            message: String::from("connection reset"),
            close_error: String::from("socket gone"),
        },
    ));
    let Err(error) = run_operation(&transport, &mut logger, unexpected).await else {
        panic!("an unexpected close failure did not surface")
    };
    assert!(matches!(
        error,
        OpenAiAssistantError::Realtime(RealtimeError::UnexpectedCloseFailure { .. })
    ));
}

pub(super) async fn case() {
    realtime_secure_session().await;
    realtime_plain_session().await;
    realtime_insecure_session().await;
    realtime_missing_key().await;
    realtime_default_base().await;
    realtime_connection_failures().await;
    realtime_close_failures().await;
}
