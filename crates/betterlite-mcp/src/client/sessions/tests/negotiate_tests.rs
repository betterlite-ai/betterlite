use std::collections::BTreeSet;

use crate::client::sessions::error::SessionError;
use crate::client::sessions::negotiate::{
    Callback, Callbacks, ClientCapabilities, InitializeOutcome as _, ProtocolVersion,
    initialize_session,
};

use super::fixture::{Config, InitStep, Script, legacy, make_config, make_plan, session, settings};

fn capabilities(kinds: BTreeSet<Callback>) -> ClientCapabilities {
    Callbacks { kinds }.capabilities()
}

#[tokio::test]
async fn auto_initialize_accepts_a_legacy_version() {
    let script = Script::default();
    let plan = make_plan(|value| value.server_version = "2025-06-18".to_owned());
    let mut session = session(&script, plan);
    let config = make_config(|value| value.protocol_version = ProtocolVersion::Auto);
    let result = initialize_session(&mut session, &settings(&config))
        .await
        .unwrap();
    assert_eq!(result.protocol_version(), "2025-06-18");
    assert_eq!(script.calls(), vec!["initialize".to_owned()]);
}

#[tokio::test]
async fn auto_initialize_rejects_a_non_legacy_version() {
    let script = Script::default();
    let plan = make_plan(|value| value.server_version = "2026-07-28".to_owned());
    let mut session = session(&script, plan);
    let config = Config::default();
    let error = initialize_session(&mut session, &settings(&config))
        .await
        .err()
        .unwrap();
    match error {
        SessionError::Protocol(failure) => {
            assert_eq!(failure.code, -32_022_i32);
        }
        other @ (SessionError::Runtime(_)
        | SessionError::Stream(_)
        | SessionError::Cancelled
        | SessionError::CleanupTimedOut
        | SessionError::CleanupTask(_)
        | SessionError::ReadTimeout(_)
        | SessionError::Configuration(_)) => {
            panic!("expected a protocol error, got {other:?}")
        }
    }
}

#[tokio::test]
async fn explicit_initialize_sends_the_configured_version_client_info_and_capabilities() {
    let script = Script::default();
    let plan = make_plan(|value| value.server_version = "2025-03-26".to_owned());
    let mut session = session(&script, plan);
    let config = make_config(|value| {
        value.protocol_version = legacy("2025-03-26");
        value.callbacks = BTreeSet::from([Callback::Sampling, Callback::Logging]);
    });
    initialize_session(&mut session, &settings(&config))
        .await
        .unwrap();
    let requests = script.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].protocol_version, "2025-03-26");
    assert_eq!(requests[0].client_info.name, "litellm");
    assert_eq!(requests[0].client_info.version, "1.105.1");
    assert_eq!(
        requests[0].capabilities,
        capabilities(BTreeSet::from([Callback::Sampling, Callback::Logging]))
    );
    assert_eq!(
        script.calls(),
        vec![
            "send_initialize".to_owned(),
            "adopt".to_owned(),
            "send_initialized".to_owned()
        ]
    );
}

#[tokio::test]
async fn explicit_initialize_rejects_a_different_server_version() {
    let script = Script::default();
    let plan = make_plan(|value| value.server_version = "2024-11-05".to_owned());
    let mut session = session(&script, plan);
    let config = make_config(|value| value.protocol_version = legacy("2025-03-26"));
    let error = initialize_session(&mut session, &settings(&config))
        .await
        .err()
        .unwrap();
    match error {
        SessionError::Protocol(failure) => assert_eq!(failure.code, -32_022_i32),
        other @ (SessionError::Runtime(_)
        | SessionError::Stream(_)
        | SessionError::Cancelled
        | SessionError::CleanupTimedOut
        | SessionError::CleanupTask(_)
        | SessionError::ReadTimeout(_)
        | SessionError::Configuration(_)) => {
            panic!("expected a protocol error, got {other:?}")
        }
    }
    assert_eq!(script.calls(), vec!["send_initialize".to_owned()]);
}

#[tokio::test]
async fn callbacks_control_the_advertised_capabilities() {
    let config = make_config(|value| {
        value.callbacks = BTreeSet::from([Callback::Sampling, Callback::Elicitation]);
    });
    let expected = ClientCapabilities {
        sampling: true,
        elicitation: true,
    };
    assert_eq!(
        capabilities(BTreeSet::from([Callback::Sampling, Callback::Elicitation])),
        expected
    );

    let script = Script::default();
    let plan = make_plan(|value| value.server_version = "2025-06-18".to_owned());
    let mut session = session(&script, plan);
    initialize_session(&mut session, &settings(&config))
        .await
        .unwrap();
    assert_eq!(script.requests()[0].capabilities, expected);
}

#[tokio::test]
async fn failed_initialize_reports_the_runtime_error() {
    let script = Script::default();
    let plan = make_plan(|value| value.initialize = InitStep::Fail);
    let mut session = session(&script, plan);
    let error = initialize_session(&mut session, &settings(&Config::default()))
        .await
        .err()
        .unwrap();
    assert!(matches!(error, SessionError::Runtime(_)));
}
