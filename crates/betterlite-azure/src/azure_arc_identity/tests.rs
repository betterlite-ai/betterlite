#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::azure_arc_identity::decode::{
    ArcReply, decode_arc_challenge, decode_arc_identity_reply,
};
use crate::azure_arc_identity::error::ArcIdentityError;
use crate::azure_arc_identity::{ArcIdentity, acquire_arc_token};
use fixture::{ENDPOINT, FixtureHost, FixtureTransport, RESOURCE, Recorded, run};

mod fixture;
mod scenarios;

#[rstest]
#[case::arc_endpoint_selection(scenarios::endpoint_selection)]
#[case::arc_challenge_request(scenarios::challenge_request)]
#[case::arc_restricted_key(scenarios::restricted_key)]
#[case::arc_second_request(scenarios::second_request)]
#[case::arc_token_reply(scenarios::token_reply)]
#[case::arc_user_identity(scenarios::user_identity)]
fn azure_arc_identity_cases(#[case] scenario: fn()) {
    scenario();
}

#[test]
fn azure_arc_identity_credential_identity_isolation() {
    let host = FixtureHost::linux(&[
        ("/var/opt/azcmagent/tokens/first.key", "first-key"),
        ("/var/opt/azcmagent/tokens/second.key", "second-key"),
    ]);
    let body = r#"{"access_token":"tok","expires_in":60}"#;
    run(&host, "Basic realm=first", body).0.unwrap();
    let (second_outcome, second) = run(&host, "Basic realm=second", body);
    second_outcome.unwrap();
    assert_eq!(
        second.recorded()[1].authorization(),
        Some("Basic second-key".to_owned())
    );

    let transport = FixtureTransport::challenge_then("Basic realm=first", body);
    let error = acquire_arc_token(
        &host,
        &transport,
        ENDPOINT,
        RESOURCE,
        ArcIdentity::UserAssigned,
    )
    .unwrap_err();
    assert!(matches!(error, ArcIdentityError::UserAssignedIdentity));
    assert!(transport.recorded().is_empty());
}

#[test]
fn azure_arc_identity_secret_safe_failure() {
    let host = FixtureHost::linux(&[("/var/opt/azcmagent/tokens/k.key", "fixture-key\n")]);
    let reply = r#"{"access_token":"fixture-token","expires_in":0,"echo":"fixture-key"}"#;
    let (rejected, sent) = run(&host, "Basic realm=k", reply);
    let error = rejected.unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(shown.contains("[redacted]"));
    assert!(!shown.contains("fixture-key"));
    assert!(!shown.contains("fixture-token"));

    let ok_body = r#"{"access_token":"fixture-token","expires_in":60}"#;
    let (accepted, _) = run(&host, "Basic realm=k", ok_body);
    let token = accepted.unwrap();
    assert_eq!(token.token.expose_secret(), "fixture-token");
    assert!(!format!("{token:?}").contains("fixture-token"));
    assert_eq!(sent.recorded()[1], Recorded::authenticated("fixture-key\n"));
    assert!(!sent.recorded()[1].url.contains("fixture-key"));
}

#[test]
fn replays_the_azure_arc_identity_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/azure_arc_identity");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            match name.split('_').next().unwrap() {
                "challenge" => assert_eq!(
                    decode_arc_challenge(&bytes).is_none(),
                    name.starts_with("challenge_bad"),
                    "{name}"
                ),
                "reply" => match decode_arc_identity_reply(&bytes) {
                    Ok(ArcReply::Token(_)) => assert!(name.starts_with("reply_ok"), "{name}"),
                    Ok(ArcReply::Rejected { .. }) => {
                        assert!(name.starts_with("reply_rejected"), "{name}");
                    }
                    Err(_value) => assert!(name.starts_with("reply_value"), "{name}"),
                },
                other => panic!("unexpected corpus prefix {other}"),
            }
            replayed += 1_usize;
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(replayed > 0_usize);
}
